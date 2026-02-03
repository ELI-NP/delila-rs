# PHA1 Full Pipeline Test Plan

**Status: COMPLETED** (2026-01-29)

## Goal
2段階でパイプラインをテスト:
1. **Phase 1**: 単一マシンでPHA1パイプラインテスト (172.18.4.147) ✅
2. **Phase 2**: マルチマシンでPSD2 + PHA1統合テスト ✅

## Prerequisites
- Machine A (macOS/local): PSD2 digitizer (VX2730, 172.18.4.56)
- Machine B (Linux, 172.18.4.147): PHA1 digitizer (DT5730B, SN: 990)
- Pulser: PHA1 Channel 4に接続済み、PSD2 Channel 16に接続済み

---

# Test Results Summary

## Phase 1: Single Machine PHA1 Test ✅

**実行日:** 2026-01-29
**環境:** 172.18.4.147 (Linux)

| Component | Events | Rate | Status |
|-----------|--------|------|--------|
| Reader (PHA1) | 29,931 | 903 evt/s | ✅ |
| Merger | 910 | - | ✅ |
| Recorder | 29,830 | 898 evt/s | ✅ |
| Monitor | 300 | - | ✅ |

**出力ファイル:** `run0001_0000_data_1769671954.delila` (513 KB)

## Phase 2: Multi-Machine Integration Test ✅

**実行日:** 2026-01-29
**環境:**
- Machine A (macOS): PSD2 Reader + Merger + Recorder + Monitor + Operator
- Machine B (Linux): PHA1 Reader

| Component | Events | Rate | Status |
|-----------|--------|------|--------|
| psd2-local | 239,348 | 8,977 evt/s | ✅ |
| pha1-remote | 29,746 | 903 evt/s | ✅ |
| Merger | 2,447 | - | ✅ |
| Recorder | 239,249 | 8,972 evt/s | ✅ |
| Monitor | 2,412 | - | ✅ |

**出力ファイル:** `run0005_0000_data.delila` (4.6 MB)

---

## Verification Checklist

### Phase 1 (Single Machine PHA1)
- [x] Reader起動・データ取得
- [x] Merger受信
- [x] Monitorヒストグラム表示
- [x] エラーなし

### Phase 2 (Multi-Machine PSD2+PHA1)
- [x] 両Reader起動
- [x] Mergerが両ソースから受信
- [x] Monitorに両チャンネル表示
- [x] 出力ファイルに両イベント
- [x] ネットワーク遅延問題なし

---

## Issues Fixed During Test

1. **PSD2 Polarity**: ch16の polarity が `Positive` になっていたため信号検出できず
   - 修正: `config/digitizers/psd2_test.json` で `"polarity": "Negative"` に変更

2. **Port 8081 Conflict**: `delila-webapi` サービスと `mongo_express` Dockerコンテナがポート8081を使用
   - 修正: サービス停止とDockerコンテナ停止

---

## Architecture (Phase 2)

```
Machine A (macOS/local)              Machine B (172.18.4.147)
┌─────────────────────────┐         ┌─────────────────────────┐
│ PSD2 Reader             │         │ PHA1 Reader             │
│ (VX2730: 172.18.4.56)   │         │ (DT5730B: USB)          │
│ PUB: tcp://*:5555       │         │ PUB: tcp://*:5556       │
│ CMD: tcp://*:5560       │         │ CMD: tcp://*:5561       │
└──────────┬──────────────┘         └──────────┬──────────────┘
           │                                    │
           │  tcp://localhost:5555              │ tcp://172.18.4.147:5556
           ▼                                    ▼
┌─────────────────────────────────────────────────────────────┐
│                    Merger (Machine A)                        │
│              subscribe: [localhost:5555, 172.18.4.147:5556] │
│              PUB: tcp://*:5557                               │
└──────────────────────────┬──────────────────────────────────┘
                           │
              ┌────────────┴────────────┐
              ▼                         ▼
┌─────────────────────┐     ┌─────────────────────┐
│ Recorder (Machine A)│     │ Monitor (Machine A) │
│ SUB: localhost:5557 │     │ SUB: localhost:5557 │
└─────────────────────┘     │ HTTP: 8081          │
                            └─────────────────────┘
```

---

## Configuration Files

### config.toml (Machine A - Local)

```toml
[operator]
experiment_name = "Multi_Digitizer_Test"

[[network.sources]]
id = 0
name = "psd2-local"
type = "psd2"
bind = "tcp://*:5555"
command = "tcp://*:5560"
digitizer_url = "dig2://172.18.4.56"
config_file = "config/digitizers/psd2_test.json"

[[network.sources]]
id = 1
name = "pha1-remote"
type = "pha1"
bind = "tcp://*:5556"
command = "tcp://172.18.4.147:5561"
digitizer_url = "dig1://caen.internal/usb?link_num=0"
config_file = "config/digitizers/pha1_test.json"

[network.merger]
subscribe = ["tcp://localhost:5555", "tcp://172.18.4.147:5556"]
publish = "tcp://*:5557"
command = "tcp://*:5570"
```

### config.toml (Machine B - Remote)

```toml
[[network.sources]]
id = 1
name = "pha1-remote"
type = "pha1"
bind = "tcp://*:5556"
command = "tcp://*:5561"
digitizer_url = "dig1://caen.internal/usb?link_num=0"
config_file = "config/digitizers/pha1_test.json"
```

---

## Important Notes

### DIG1 vs DIG2 Protocol
- **PSD2 (dig2)**: N_EVENTS あり、Arm → Start 分離
- **PHA1 (dig1)**: N_EVENTS なし、Arm = Start (START_MODE_SW)

### PHA1 License
- 30分でライセンス切れ → デジタイザ再起動必要

### Network Requirements
- Machine A → Machine B: port 5556 (data), 5561 (command)
- Firewall: ZMQ ports (5555-5591) を開放
