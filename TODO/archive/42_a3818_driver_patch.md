# TODO #42: A3818 PCIe CONET2 Driver Patch

**Status: COMPLETED**
**Created:** 2026-02-24
**Completed:** 2026-02-24

## Summary

CAEN A3818 PCIe CONET2 ドライバ (v1.6.12) のカーネルパニックを引き起こす複数のバグを修正。
パッチ版 `v1.6.12-delila1` を作成し、172.18.4.76 にデプロイ済み。

## Background

- 高レート PSD (>500K events/s) でカーネルパニックが発生
- PHA でも高レート・高ノイズ時に発生
- Gemini Deep Research + 静的コード解析で根本原因を特定

## Analysis

詳細解析: [docs/a3818_driver_analysis.md](../docs/a3818_driver_analysis.md)

6 件のバグを発見（うち 5 件を修正）:

| # | Severity | Bug | Fixed |
|---|----------|-----|-------|
| 1 | CRITICAL | `a3818_dispatch_pkt()` 1MB バッファオーバーフロー (境界チェックなし) | Yes |
| 2 | HIGH | 割り込みハンドラ off-by-one (`NumOfLink` → `NumOfLink-1`) | Yes |
| 3 | HIGH | IOCTL_SEND `err_send` セマフォリーク (永久デッドロック) | Yes |
| 4 | MEDIUM | IOCTL_RECV 不均衡な `up()` (排他制御無効化) | Yes |
| 5 | MEDIUM | 0xFFFFFFFF PCIe デバイス消失時の writel → MCE | Yes |
| 6 | LOW | `a3818_mmiowb()` 空定義 (x86 では実害なし) | No |

## Root Cause of Kernel Panic

`a3818_dispatch_pkt()` 内の `app_dma_in` (1MB vmalloc) バッファに対する境界チェックの欠如。
PSD で 524K events/s (100ms readout) を超えると 1MB を超過し、vmalloc ガードページへの
memcpy が割り込みコンテキスト内で発生 → 即座にカーネルパニック。

## Patches Applied

Source: `external/a3818_linux_driver-v1.6.12/src/a3818.c`
Version: `v1.6.12-delila1`

1. **バッファオーバーフロー防止** + vmalloc 1MB→16MB (`APP_DMA_IN_SIZE`)
2. **割り込みハンドラ off-by-one 修正**: `for(i = s->NumOfLink - 1; i >= 0; i--)`
3. **IOCTL_SEND セマフォリーク修正**: 成功・エラー両パスで統一的に `up()` を呼ぶ
4. **IOCTL_RECV 不均衡 `up()` 削除**: `down()` なしの `up()` を除去
5. **0xFFFFFFFF 検出時の即時エラー**: `deviceDead` フラグ → `return -EIO` (writel 到達防止)

## Deployment

- **Deploy先:** `daq@172.18.4.76:~/a3818_linux_driver-v1.6.12-delila1/`
- **ビルド:** `make` (warnings=0, errors=0)
- **ロード:** `rmmod a3818 && insmod a3818.ko` (手動)
- **確認:** `/proc/a3818` → `v1.6.12-delila1`, 4 links, all working
- **注意:** 172.18.4.76 のみ A3818 あり。172.18.4.147 にはなし。

## Future Work

- CAEN サポートに Bug #1-4 を報告 (support.computing@caen.it)
- Bug #6 (`a3818_mmiowb()`) は x86 では実害なしのため未修正
- 高レートテストで kernel panic が再発しないことを確認
