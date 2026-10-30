# taikosim

Linux / Windows で動く、KISS 方針の太鼓系リズムゲームシミュレータです。  
**Rust で実装**し、ダウンロード済み `.tja` を選曲して遊べます。

- `.tja` を読み込んでプレイ
- GUI アプリ（選曲 -> プレイ -> リザルト）
- ネットから取得した譜面をローカル配置してそのまま遊べる

## クイックスタート

## 1) 必要環境（Linux）

- Rust (stable)
- Linux の一般的な音声環境（PulseAudio/PipeWire）
- ビルドに必要なシステムパッケージ: C リンカ、`pkg-config`、ALSA 開発ヘッダ
- 実行時に必要なライブラリ: X11 / Xi / OpenGL（Wayland 環境では EGL / wayland-client / xkbcommon）。デスクトップ環境なら通常は導入済みです。

### Rust のインストール

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### システムパッケージ

Ubuntu / Debian:

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libasound2-dev
# Runtime libraries (usually already installed on desktops)
sudo apt-get install -y libx11-6 libxi6 libgl1
```

Fedora:

```bash
sudo dnf install -y gcc pkgconf-pkg-config alsa-lib-devel
```

Arch Linux:

```bash
sudo pacman -S --needed base-devel pkgconf alsa-lib
```

### ビルド確認

```bash
git clone https://github.com/okayu0518/taiko-sim.git
cd taiko-sim
cargo build
cargo test
```

## Windows用exe

GitHub Actions の **Actions → CI → 対象の実行 → Artifacts → taikosim-windows-x64** から配布ZIPを取得できます。ZIPを展開し、`taikosim/taikosim.exe` を起動します。利用者にはRustのインストールは不要です。Windows 11上での動作は実機で確認してください。

配布用のフォルダ構成:

```text
taikosim/
  taikosim.exe
  config.json
  assets/
    fonts/
    se/
  data/
    songs/
  README.md
```

配布版はexeの隣の `assets/`, `data/`, `config.json` を優先します。`cargo run` ではプロジェクトの作業ディレクトリからも読み込めます。 `config.example.json` は配布用の初期設定です。

Windows上で直接ビルドする場合はRust（MSVC）とVisual Studio C++ Build Tools（Desktop development with C++）をインストールし、PowerShellから `cargo build --release` を実行してください。

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
  "se_enabled": true,
  "lead_in_sec": 3.0
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
- `lead_in_sec`: 曲が始まるまでの猶予（秒、0〜10）。この間にノーツが流れてくる

`debug: true` のとき、プレイ画面右上に SE 遅延の簡易計測値を表示します。

- `last`: 直近入力からSEサンプルが初めて読み出されるまでの時間（ms）。実際のスピーカー出力遅延ではありません。
- `avg`: 平均（ms）
- `max`: 最大（ms）

## 入力について

左右Don（F/J）とKat（D/K）は個別に処理し、左右の同時押しも別々の入力になります。ただし `macroquad::is_key_pressed` を使うため、同一キーを1フレーム中に2回押しても2回の入力とは認識されません。SEの再生はサンプルレートとチャンネル数を保持してステレオ音源を正しく扱います。

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
  - コマンド対応: `#BPMCHANGE`, `#SCROLL`, `#MEASURE`, `#DELAY`, `#GOGOSTART`, `#GOGOEND`（`#MEASURE` 以外は小節途中でも可）
  - 譜面分岐: `#BRANCHSTART`（`r` 連打数 / `p` 精度 / `s` スコア）, `#N`, `#E`, `#M`, `#BRANCHEND`, `#SECTION`, `#LEVELHOLD`
- スクロール: ノーツごとの `BPM × SCROLL` に比例した速さで流れる（`#SCROLL 1` で 1 拍 = 240px）
- 譜面分岐の判定: 分岐の 1 小節前の小節線で、前回の判定（または `#SECTION`）以降の成績から決定
- 判定: 良 `±25ms` / 可 `±75ms` / 不可（おに相当で固定、大音符も通常ノーツと同じ扱い）
  - 可の範囲を過ぎて叩かれなかったノーツは不可
  - 色違いの入力や判定範囲外の入力は無視（ノーツは消費しない）
  - スコア: 良 `300` / 可 `150` / 不可 `0`、不可でコンボが切れる
- ヒットエフェクト: 入力時のレーン発光、ヒットバースト、ノーツが飛ぶ演出、判定文字
- スコア/コンボ/リザルト表示
- ゴーゴータイム: ノーツ・連打の得点が 1.2 倍。判定枠に区間が来るとレーンがオレンジになり、判定枠の周りに炎を表示
- 連打ノーツ: `5/6/7/9` 開始、`8` 終了をパースして描画/加点
- タイミング補正: `timing_offset_ms` を保存して再起動後も維持
- 連打中は判定枠上に連打数を表示

## 開発

### テスト

```bash
cargo test                 # all unit tests
cargo test gameplay        # filter by module/test name
```

- テストは各モジュール内の `#[cfg(test)] mod tests` にあります。`data/` には依存しません。
  - `judge.rs`: 判定幅・スコア
  - `gameplay.rs`: 判定・見逃し・連打・分岐（`PlaySession`）
  - `tja_parser.rs`: ヘッダ・コース選択・文字コード・ノーツ時刻・命令・分岐
  - `repository.rs`: 曲フォルダ走査・`index.json`（一時ディレクトリを使用）
  - `effects.rs`: エフェクトの寿命管理
- ゲーム進行ロジックは描画・音声・入力から独立した `PlaySession`（`src/gameplay.rs`）にあります。判定や分岐を変更する場合はここにテストを追加してください。
- 未対応仕様をテストで残す場合は `#[ignore = "理由"]` を付け、`cargo test -- --ignored` で確認できるようにしてください。

### CI

GitHub ActionsはUbuntuのビルド・テストに加え、Windows MSVCでのテスト・Releaseビルドと配布ZIPの生成を実行します。

## 文字コード対応

`.tja` は以下を自動判定で読み込みます。

- UTF-8（BOM含む）
- Shift_JIS（CP932系）

## インターネット譜面の扱い

- このアプリは「**すでにダウンロード済み**」の譜面/音源を再生対象にします。
- まず譜面を取得し、`data/songs/<曲名>/` に配置してください。

## 注意

- 譜面・音源の利用は著作権と配布元ルールを遵守してください。
