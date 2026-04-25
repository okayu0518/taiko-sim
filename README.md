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

## 操作

- 選曲画面
  - `↑/↓`: 曲選択
  - `Enter`: 開始
  - `Esc`: 終了
- プレイ画面
  - `F`, `J`: Don
  - `D`, `K`: Kat
  - `Esc`: リザルトへ
- リザルト画面
  - `Enter`: 選曲へ戻る
  - `Esc`: 終了

## 実装済み範囲（Rust版MVP）

- 曲一覧: `data/songs/*` 自動スキャン
- `.tja` パーサ:
  - 難易度選択: `Oni` 優先（なければ `Hard` -> `Normal` -> `Easy`）
  - 対応: `TITLE`, `WAVE`, `BPM`, `OFFSET`, `COURSE`, `LEVEL`
  - コマンド対応: `#BPMCHANGE`, `#MEASURE`, `#DELAY`
- 判定: `Perfect/Good/Miss`
- スコア/コンボ/リザルト表示

## 文字コード対応

`.tja` は以下を自動判定で読み込みます。

- UTF-8（BOM含む）
- Shift_JIS（CP932系）

## インターネット譜面の扱い

- このアプリは「**すでにダウンロード済み**」の譜面/音源を再生対象にします。
- まず譜面を取得し、`data/songs/<曲名>/` に配置してください。

## 注意

- 譜面・音源の利用は著作権と配布元ルールを遵守してください。
