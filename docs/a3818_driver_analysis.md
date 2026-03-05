# CAEN A3818 PCIe CONET2 ドライバ (v1.6.12) カーネルパニック解析レポート

**解析日:** 2026-02-24
**対象:** `external/a3818_linux_driver-v1.6.12/src/a3818.c`
**解析ツール:** Gemini Deep Research + 静的コード解析 + バッファオーバーフロー計算

## Executive Summary

CAEN A3818 PCIe CONET2 ドライバ (v1.6.12) には、高イベントレート時にカーネルパニックを引き起こす複数の重大なバグが存在する。最も可能性が高い原因は、`a3818_dispatch_pkt()` 内の **1MB vmalloc バッファに対する境界チェックの欠如** によるヒープ破壊である。PSD ファームウェアで 700K events/s を超えるレートでは、1回のリードアウトで 1MB を超えるデータが蓄積し、vmalloc ガードページへの書き込みが割り込みコンテキスト内で発生してカーネルパニックとなる。

---

## 発見されたバグ（深刻度順）

### Bug 1: [CRITICAL] `app_dma_in` バッファオーバーフロー

**場所:** `a3818_dispatch_pkt()` (a3818.c:549)
**高レート時カーネルパニックの最有力原因**

```c
pos = s->pos_app_dma_in[opt_link][slave];
// ★ pos が 1MB を超えていないか一切チェックしていない！
memcpy(&(iobuf[pos]), &(buff_dma_in[i]), pkt_sz * 2);
s->pos_app_dma_in[opt_link][slave] += pkt_sz * 2;
```

**メカニズム:**
- `app_dma_in` は **1MB** (`vmalloc(1024*1024)`, line 1449)
- 個別パケットは `pkt_sz <= 0x100` (512バイト) だが、**累積チェックなし**
- 複数の DMA 転送（各最大 128KB）がひとつの IOCTL_COMM レスポンスに蓄積される
- `pos_app_dma_in` は IOCTL_COMM 完了時（`err_comm` ラベル, line 917）でのみリセットされる

**オーバーフロー閾値の計算:**

| リードアウト間隔 | PSD (20B/evt) でオーバーフローするレート | PHA (18B/evt) |
|:---:|:---:|:---:|
| 50ms | ~1,049K events/s | ~1,165K events/s |
| 100ms | **~524K events/s** | ~582K events/s |
| 200ms | ~262K events/s | ~291K events/s |
| 500ms | ~105K events/s | ~116K events/s |

波形データ付き（512サンプル）の場合、閾値は **~10K events/s** まで低下する。

**実測シナリオ:**
- PSD1 で 700K events/s → 100ms リードアウトで約 1.4MB → 1MB バッファを **40% 超過**
- vmalloc はガードページを持つため、1MB を超えた memcpy → ページフォルト
- 割り込みコンテキスト内のページフォルトは **即座にカーネルパニック**

**パニックのフロー:**
```
高イベントレート → DMA 転送が高頻度
→ 割り込みハンドラが a3818_dispatch_pkt を頻繁に呼ぶ
→ app_dma_in (1MB) に蓄積、pos_app_dma_in が増大
→ ユーザースペースが読み出す前に 1MB を超過
→ memcpy が vmalloc ガードページに書き込み
→ 割り込みコンテキストでのページフォルト
→ ★ KERNEL PANIC ★
```

### Bug 2: [HIGH] 割り込みハンドラの off-by-one

**場所:** `a3818_interrupt()` (a3818.c:1180)

```c
for( i = s->NumOfLink; i > -1 ; i-- ) {
    // ★ NumOfLink (e.g., 4) から開始 — 0-indexed 配列なのに！
    if( app & (A3818_DMISCCS_RDDMA_DONE0 << i) ) {
        writel(A3818_RES_DMAINT, s->baseaddr[i] + A3818_DMACSR_S);
        // baseaddr[NumOfLink] は ioremap されていない（NULL）
```

**影響:**
- 4-link カード: `NumOfLink=4` → `baseaddr[4]` は未マッピング (memset で 0 初期化 = NULL)
- `DataLinklock[4]` は未初期化（配列サイズ `MAX_OPT_LINK=5` なので配列内だが spin_lock_init されていない）
- 通常は bit20 (`RDDMA_DONE4`) がセットされないため発動しないが、ハードウェアエラー時やノイズ時にセットされる可能性あり
- 発動した場合: NULL ポインタ逆参照 → カーネルパニック

**配列サイズ確認:**
```c
// a3818.h
#define MAX_OPT_LINK (0x05)

unsigned char *baseaddr[MAX_OPT_LINK + 1];  // [6] — index 0-5
spinlock_t     DataLinklock[MAX_OPT_LINK];  // [5] — index 0-4
unsigned int   DMAInProgress[MAX_OPT_LINK]; // [5] — index 0-4
```

- `baseaddr[4]` → 配列内だが ioremap されていない
- `DataLinklock[4]` → 配列内だが spin_lock_init されていない（4-link カードでは 0-3 のみ初期化）
- 1-link/2-link カードでは `baseaddr[1]`/`baseaddr[2]` も NULL → 確実にクラッシュ

### Bug 3: [HIGH] IOCTL_SEND のセマフォリーク → 永久デッドロック

**場所:** `a3818_ioctl()` case IOCTL_SEND (a3818.c:1049-1073)

```c
case IOCTL_SEND:
    down(&s->ioctl_lock[opt_link]);     // ロック取得
    // ...
    if( readl(...) & A3818_LINK_FAIL ) {
        a3818_reset_comm(s, opt_link);
        mdelay(10);
        if( readl(...) & A3818_LINK_FAIL ) {
            ret = -EACCES;
            goto err_send;              // ★ up() せずに脱出！
        }
    }
    // ... success path ...
    up(&s->ioctl_lock[opt_link]);       // 正常時はここで解放
    break;

err_send:
    break;  // ★ セマフォが永久にロックされたまま → そのリンクは二度と使えない
```

光リンクエラーが1回発生するだけで、そのリンクは永久にデッドロックする。

### Bug 4: [MEDIUM] IOCTL_RECV のアンバランスなセマフォ

**場所:** `a3818_ioctl()` case IOCTL_RECV (a3818.c:1075-1083)

```c
case IOCTL_RECV:
    if ((s->NumOfLink == 0) && ...)
        return -EFAULT;
    ret = a3818_recv_pkt(s, slave, opt_link, (int *)arg);
    up(&s->ioctl_lock[opt_link]);   // ★ down() していないのに up()!
    if( ret < 0 ) {
        ret = -EFAULT;
    }
    break;
```

呼ぶたびにセマフォカウントが増加 → `ioctl_lock` が排他制御として機能しなくなる → IOCTL_COMM の同時実行でハードウェア状態破壊、データ破壊の可能性。

### Bug 5: [MEDIUM] 0xFFFFFFFF 読み取り後のデッドデバイスアクセス

**場所:** `a3818_recv_pkt()` (a3818.c:394-411)

```c
tr_stat = readl(s->baseaddr[opt_link] + A3818_LINK_TRS);
if( tr_stat == 0xFFFFFFFF ) {
    DPRINTK("rcv-pkt: Error: tr_stat = %x\n", s->tr_stat[opt_link]);
    break;  // ★ 内部ループを抜けるだけ — エラー状態を設定しない
}
// ... ループ後 ...
if (!startDMA) ENABLE_RX_INT(opt_link);
// ★ ENABLE_RX_INT は writel() → 死んだデバイスに書き込み → Machine Check Exception
```

PCIe デバイスが応答しない状態（0xFFFFFFFF = デバイス消失）で writel すると MCE → カーネルパニック。

### Bug 7: [CRITICAL] `a3818_open()` の spin_lock + msleep → scheduling while atomic

**場所:** `a3818_open()` (a3818.c:784-790)
**発見日:** 2026-03-05
**発見契機:** 172.18.4.76 が 2026-03-04 17:54 にフリーズ。翌日の再起動でも segfault。

```c
if( s->TypeOfBoard == A3818BOARD ) {
    if (!s->GTPReset) {
        spin_lock( &s->CardLock);                      // ★ preempt_count += 1
        if (a3818_reset_onopen(s) == A3818_OK)         // ★ 内部で msleep(10) + msleep(1)!
            s->GTPReset = 1;
        spin_unlock( &s->CardLock);
    }
}
```

**メカニズム:**
- `spin_lock()` は preemption を無効化する (preempt_count > 0 → "atomic" コンテキスト)
- `a3818_reset_onopen()` 内で `msleep(10)` + `msleep(1)` を呼ぶ
- `msleep()` → `schedule()` → カーネルが preempt_count > 0 を検出 → `BUG: scheduling while atomic`
- さらに `spin_lock` (IRQ 無効化なし) なので、同一 CPU で IRQ が発火すると CardLock デッドロック

**再現シナリオ (再接続ストーム):**
1. A3818 光リンク一時障害 → 10/12 Reader が同時に CAEN error -6
2. 30s トランジエントエラーリトライ失敗 → 全 Reader が Error 遷移 → connection = None
3. 1 秒間隔の再接続ループで全 Reader が `a3818_open()` → `spin_lock` + `msleep` を並列実行
4. `BUG: scheduling while atomic` → workqueue CPU hogging → RT throttling → システムフリーズ

**実測データ (2026-03-05 dmesg):**
```
BUG: scheduling while atomic: tokio-runtime-w/26020/0x00000002
 a3818_open+0x176/0x3d0 [a3818]
BUG: scheduling while atomic: tokio-runtime-w/26050/0x00000002
 a3818_open+0x176/0x3d0 [a3818]
BUG: scheduling while atomic: tokio-runtime-w/26085/0x00000002
 ...
a3818: rcv-pkt: Timeout on RX -> Link 0  (6回、回復せず)
```

**修正:** `v1.6.12-delila2` で対応
- `CardLock` (spinlock) → 専用 `GTPResetMutex` (mutex) に変更
- Mutex はスリープ可能 → `msleep` が安全に呼べる
- IRQ ハンドラは `CardLock` を使い続ける (変更なし)
- Double-checked locking で GTPReset の一回限り実行を保証

### Bug 6: [LOW] `a3818_mmiowb()` が空定義

**場所:** a3818.c:64

```c
#define a3818_mmiowb()  // 何もしない
```

MMIO 書き込み後のオーダリングバリアがない。x86 では通常問題にならないが（強い順序保証がある）、DMA レジスタ設定後に DMA 開始する場合のレースの可能性がある。

---

## ロックオーダリング分析

```
割り込みハンドラ:
  spin_lock(&CardLock)           → Lock A
    spin_lock(&DataLinklock[i])  → Lock B
    spin_unlock(&DataLinklock[i])
  spin_unlock(&CardLock)

recv_pkt:
  spin_lock_irq(&DataLinklock[i])  → Lock B (IRQ 無効)
    // ポーリング + handle_rx_pkt
  spin_unlock_irq(&DataLinklock[i])
  // ENABLE_RX_INT
  wait_event_interruptible_timeout(...)  // スリープ
```

Lock B → Lock A の逆順はないため AB-BA デッドロックは発生しない。ただし SMP 環境で以下の競合がある:
- CPU0: recv_pkt が DataLinklock を保持中に DMA 開始
- CPU1: DMA 完了割り込み → CardLock 取得 → DataLinklock 取得試行 → スピン待ち
- CPU0 が DataLinklock 解放するまで CPU1 はスピン → デッドロックではないが高レートで性能劣化

---

## 推奨対策

### 優先度 1: バッファオーバーフロー修正

```c
// a3818_dispatch_pkt() 内、memcpy の前に追加:
#define APP_DMA_IN_SIZE (1024 * 1024)

pos = s->pos_app_dma_in[opt_link][slave];
int bytes_to_copy = pkt_sz * 2;
if (pos + bytes_to_copy > APP_DMA_IN_SIZE) {
    printk(KERN_ERR PFX "Buffer overflow prevented on link %d slave %d (pos=%d, copy=%d)\n",
           opt_link, slave, pos, bytes_to_copy);
    return;  // パケットを破棄
}
memcpy(&(iobuf[pos]), &(buff_dma_in[i]), bytes_to_copy);
```

さらに、バッファサイズを増大:
```c
// a3818_init_board() の vmalloc を変更:
// 変更前:
s->app_dma_in[i][j] = (u8 *)vmalloc(1024*1024);
// 変更後:
s->app_dma_in[i][j] = (u8 *)vmalloc(16*1024*1024);  // 16MB
```

### 優先度 2: ループ修正

```c
// 変更前:
for( i = s->NumOfLink; i > -1 ; i-- ) {
// 変更後:
for( i = s->NumOfLink - 1; i >= 0 ; i-- ) {
```

### 優先度 3: セマフォ修正

**IOCTL_SEND の err_send:**
```c
err_send:
    up(&s->ioctl_lock[opt_link]);  // 追加
    break;
```

**IOCTL_RECV の余分な up() 削除:**
```c
case IOCTL_RECV:
    if ((s->NumOfLink == 0) && ...)
        return -EFAULT;
    ret = a3818_recv_pkt(s, slave, opt_link, (int *)arg);
    // up(&s->ioctl_lock[opt_link]);  // 削除
    if( ret < 0 ) {
        ret = -EFAULT;
    }
    break;
```

### 優先度 4: 0xFFFFFFFF 検出時の即時エラー

```c
// a3818_recv_pkt() 内:
tr_stat = readl(s->baseaddr[opt_link] + A3818_LINK_TRS);
if( tr_stat == 0xFFFFFFFF ) {
    printk(KERN_ERR PFX "PCIe device failure on link %d\n", opt_link);
    spin_unlock_irq(&s->DataLinklock[opt_link]);
    return -EIO;  // writel() に到達させない
}
```

### 優先度 5: ユーザースペース側の緩和策

ドライバを修正できない場合の暫定対策:
- リードアウト間隔を短くして 1回のデータ量を 1MB 以下に抑える（50ms 以下推奨）
- 波形データ取得を高レート時は無効にする
- EventsPerInterrupt / BLT_SIZE を調整して DMA 転送頻度を下げる

---

## 参考情報

### 他プロジェクトでの類似問題

- **FSUDAQ** (FSU DAQ framework): A3818 Driver v1.6.8 で高入力レート (>60kHz) 時のパイルアップとエラーを報告
- **Kodiaq** (XENON experiment): A3818/V1724 セットアップで segfault と "insufficient BLT buffer size" を報告
- **MIDAS DAQ**: ドライバ内に `#define USE_MIDAS 0/1` の切り替えがあり、`wait_event_timeout`（シグナル無視）を使用するモードが存在。シグナルによる中断を防止する目的。

### CAEN サポートへの報告推奨事項

以下の情報と共に CAEN サポート (support.computing@caen.it) にバグレポートを送ることを推奨:
1. `a3818_dispatch_pkt()` のバッファ境界チェック欠如
2. 割り込みハンドラのループ off-by-one
3. IOCTL_SEND/IOCTL_RECV のセマフォバグ
4. カーネルパニックの再現条件（DT5730B, PSD FW, >500K events/s）
