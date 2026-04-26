# taikosim

Linux で動く、KISS 方針の太鼓系リズムゲームシミュレータです。  
**Rust で実装**し、ダウンロード済み `.tja` を選曲して遊べます。

- `.tja` を読み込んでプレイ
- GUI アプリ（選曲 -> プレイ -> リザルト）
- ネットから取得した譜面をローカル配置してそのまま遊べる

## クイックスタート

## 1) 必要環境

- Rust (stable)
- Linux の一般的な音声環境（PulseAudio/PipeWire）

## 2) サンプル譜面配置

```text
data/
  songs/
    森羅万象/
      森羅万象.tja
      森羅万象.ogg
```

`data/songs/<曲フォルダ>/` に `.tja` と音源（`.ogg/.wav/.mp3`）を置けば自動検出されます。

## 3) 起動

```bash
cargo run
```

## 日本語フォント

- `assets/fonts/NotoSansCJK-Regular.ttc` を同梱しています（推奨）。
- ライセンスは `assets/fonts/LICENSES/OFL.txt` を参照してください。
- 追加で他フォントを使う場合は、以下も探索します。
  - `assets/fonts/NotoSansJP-Regular.ttf`
  - `assets/fonts/NotoSansCJKjp-Regular.otf`
- Linux システムフォントも自動探索します（例: `/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc`）。
- または `config.json` に `font_path` を指定できます（リポジトリルートからの相対パス）。

```json
{
  "timing_offset_ms": 0,
  "se_volume": 0.9,
  "music_volume": 1.0,
  "keybind": {
    "don_left": "F",
    "don_right": "J",
    "kat_left": "D",
    "kat_right": "K"
  },
  "vsync": true,
  "font_path": "assets/fonts/NotoSansCJK-Regular.ttc",
  "debug": false,
  "se_enabled": true
}
```

- `timing_offset_ms`: 譜面タイミング補正（ms）
- `se_volume`: 効果音音量（0.0〜2.0）
- `music_volume`: 楽曲音量（0.0〜2.0）
- `keybind`: Don/Kat のキー割り当て（`A-Z`, `LEFT`, `RIGHT`, `UP`, `DOWN`, `SPACE`, `ENTER`, `TAB` など）
- `vsync`: 垂直同期（起動時に反映）
- `font_path`: 使用フォントパス（省略時は同梱フォント優先）
- `debug`: デバッグ表示（FPS/Time）
- `se_enabled`: 入力SEのON/OFF

`debug: true` のとき、プレイ画面右上に SE 遅延の簡易計測値を表示します。

- `last`: 直近入力のSE再生開始までの時間（ms）
- `avg`: 平均（ms）
- `max`: 最大（ms）

## 操作

- 選曲画面
  - `↑/↓`: 曲選択
  - `Enter`: 開始
  - `Esc`: 終了
- プレイ画面
  - `F`, `J`: Don
  - `D`, `K`: Kat
  - `[`, `]`: 譜面タイミング補正（-5ms / +5ms）
  - `Esc`: リザルトへ
- リザルト画面
  - `Enter`: 選曲へ戻る
  - `Esc`: 終了

## 実装済み範囲（Rust版MVP）

- 曲一覧: `data/songs/*` 自動スキャン（同一フォルダに `.tja` が複数ある場合は全て読み込み）
- `.tja` パーサ:
  - 難易度選択: `Oni` 優先（なければ `Hard` -> `Normal` -> `Easy`）
  - 対応: `TITLE`, `WAVE`, `BPM`, `OFFSET`, `COURSE`, `LEVEL`
  - コマンド対応: `#BPMCHANGE`, `#MEASURE`, `#DELAY`
- 判定: `Perfect/Good/Miss`
- スコア/コンボ/リザルト表示
- 連打ノーツ: `5/6/7/9` 開始、`8` 終了をパースして描画/加点
- タイミング補正: `timing_offset_ms` を保存して再起動後も維持
- 空打ちは Miss にしない（判定範囲外入力は無視）
- 連打中は判定枠上に連打数を表示

## 文字コード対応

`.tja` は以下を自動判定で読み込みます。

- UTF-8（BOM含む）
- Shift_JIS（CP932系）

## インターネット譜面の扱い

- このアプリは「**すでにダウンロード済み**」の譜面/音源を再生対象にします。
- まず譜面を取得し、`data/songs/<曲名>/` に配置してください。

## 注意

- 譜面・音源の利用は著作権と配布元ルールを遵守してください。
