# TODO 50: macOS userspace USB CDC-EEM driver (libusb + utun)

**Status:** PLANNED (post-MVP, low priority — fun project)
**Created:** 2026-04-30
**Owner:** TBD (good weekend project for whoever wants it)

## ゴール

CAEN VX2730 (および将来の CDC-EEM 対応 CAEN dig2 機材) を **macOS から USB 直結で使えるようにする**。現状 USB 接続は Ethernet 経由 1 Gb の天井 (944 Mbps) を超える唯一のルートだが、macOS Tahoe 以降 native EEM サポートがないため使えない。userspace ドライバを書いて穴を埋める。

## 動機

PSD2 throughput sweep (2026-04-30) で確認:
- 1 Gb Ethernet line rate (944 Mbps) が VX2730 の絶対天井
- 600 samples/event で 43 kHz/board が saturation, 70% 利用で 30 kHz が安全運転
- VX2730 USB spec は 280 MB/s ≒ 2.2 Gbps, **Ethernet の 2.4×**
- 単 board の高レート運用 or 短ケーブル便利テストの両方で USB が活きる

## 背景 — なぜ macOS で動かないか

- VX2730 は USB に **3 つの interface** を expose:
  1. RNDIS Communications Control (Class=2, SubClass=2, Protocol=0xFF) — Microsoft RNDIS
  2. RNDIS Ethernet Data (Class=10)
  3. **CDC EEM (Class=2, SubClass=12, Protocol=7)** — USB-IF 標準
- macOS Tahoe 26.x で `AppleUSBEEM.kext` は **codeless** になっている (Info.plist のみ、binary 無し)
  - `kmutil load` → "Bad code signature" でエラー
  - `kmutil inspect` の kernel collection に `cdc.eem` が **無い** (acm/ecm/ncm はある)
  - DriverKit Extension (DEXT) 後継も無し
- RNDIS 側も macOS は native ドライバを持たない (HoRNDIS は EOL)

→ **OS から修正を待つ道は塞がれている**。userspace で実装するしかない。

詳細経緯: `~/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/mac_usb_eem_status.md`

## アプローチ

**libusb で USB CDC-EEM interface を claim → utun で macOS のネットワークスタックに inject** する userspace network driver。Linux カーネル `drivers/net/usb/eem.c` (~300 行) が完全な reference 実装。

### 技術コンポーネント

| 要素 | 担当 |
|---|---|
| USB endpoint I/O | `rusb` crate (libusb wrapper) |
| EEM frame encode/decode | 自前 (16-bit header: D bit + length, optional CRC32) |
| Kernel network injection | `tun` crate (utun on macOS) |
| MTU 9000 / 15000 | utun は対応、ifconfig で設定 |
| IPv6 SLAAC | OS 任せ (utun に prefix が来れば自動) |
| mDNS (`CAENDGTZ-USB-{SN}.local`) | macOS の mDNSResponder が自動処理 |
| FELib2 接続 | `dig2://CAENDGTZ-USB-{SN}.local` でそのまま |

### EEM プロトコルの簡潔さ

EEM frame format (USB-IF EEM spec):
```
Bit 15:    bmType   (0=Data, 1=Command)
Bit 14:    bmCRC    (data frame: 1=CRC32 present, 0=sentinel CRC)
Bit 13-0:  Length   (data frame: payload length, command frame: opcode-specific)
[Payload]
[CRC32 if Data && bmCRC=1]
```

実装は 200〜300 行で済む。Linux 実装と等価のものを Rust + 安全な libusb で書き直す。

### 推定工数

- **Phase 1 (PoC)**: 週末 1 回 — frame encode/decode, libusb ループ, utun に出すだけ。ping6 が通れば Win
- **Phase 2 (本番化)**: 週末もう 1 回 — エラーハンドリング, 切断検出, MTU 動的設定, ホットプラグ, signal handling
- **Phase 3 (delila-rs 統合)**: 数時間 — `dig2://...` から接続できることの実機確認, throughput 比較ベンチ

## 検証可能なゴール (Definition of Done)

1. ✅ `delila-usb-eem-driver` (新 binary) を起動すると `utun{N}` インターフェイスが生える
2. ✅ MTU 9000 以上で動作 (jumbo)
3. ✅ `ping6 CAENDGTZ-USB-52622.local` が通る
4. ✅ `target/release/reader --config <USB-config>` で digitizer に dig2 接続成功
5. ✅ throughput sweep で USB が 1 Gb Ethernet を上回ることを実測 (target ≥ 200 MB/s sustained, vs 117 MB/s limit on 1 Gb)
6. ✅ Stop / re-plug で起動中のドライバが再接続できる

## 参考

- Linux kernel reference: `drivers/net/usb/eem.c`, `include/linux/usb/cdc.h`
- USB EEM spec: USB-IF "Communication Class Subclass Specification for Ethernet Emulation Model Devices" (公開)
- macOS utun: System Configuration framework, `if_utun.h` (一応 SPI), or `tun-tap` Rust crate (高レベル)
- 動作確認済み代替 (Linux VM 経由): 詳細は memory/mac_usb_eem_status.md
- Rust libusb wrapper: `rusb` crate (https://crates.io/crates/rusb)

## なぜ低優先

- **Ethernet で MVP は十分動いている** — 30 kHz × 数 board は line rate 内、安全係数込みで運用可能
- **USB の優位性は単 board 高レート / Mac での portable testing 用途**
- **ポスト MVP の "あったら嬉しい" カテゴリ**
- ただし「Mac のラボベンチで VX2730 をサクッと触る」体験は工数の割に効用が大きいので、隙間時間にやる価値あり

## アーカイブ条件

- 着手しないまま 1 年以上経過 → archive へ
- 着手して PoC が通った → Status: COMPLETED に更新、CURRENT.md "Recently Completed" へ
