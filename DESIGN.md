# DESIGN

このドキュメントは、Linux 向け太鼓系リズムゲームシミュレータの実装設計です。  
方針は **KISS**。複雑さを避け、遊べる最短経路を優先します。

## 1. 要件整理

### 機能要件

- `.tja` を読み込んでプレイできる
- GUI アプリである
- インターネットから譜面リストを取得できる
- 楽曲一覧を表示し、キーボードで選択できる
- 基本的な判定・スコア表示がある

### 非機能要件

- Linux 上で動作（X11/Wayland どちらでも通常動作）
- 依存を少なく保つ
- 起動失敗しにくい（不足ファイル時のエラー表示）

## 2. アーキテクチャ概要

単一プロセス・単一ゲームループ構成。

- `App`
  - 初期化、メインループ、シーン管理
- `Scene`（抽象）
  - `handle_event`, `update`, `render`
- `SongSelectScene`
  - リモート index の取得、曲一覧、選択
- `GameplayScene`
  - `.tja` 読み込み、譜面進行、判定、スコア
- `ResultScene`
  - リザルト表示、再挑戦/戻る

補助モジュール:

- `TJAParser`: `.tja` を内部譜面モデルへ変換
- `Judge`: 入力タイミング判定
- `AudioPlayer`: BGM 再生と再生時刻取得
- `Downloader`: `index.json` と譜面/音源の取得
- `CacheStore`: ローカル保存・読み込み

## 3. データモデル

## 3.1 SongMeta

- `id: str`
- `title: str`
- `tja_url: str`
- `audio_url: str`
- `local_tja_path: str | None`
- `local_audio_path: str | None`

## 3.2 Chart

- `title: str`
- `bpm: float`
- `offset_sec: float`
- `course: str`
- `level: int`
- `notes: list[Note]`

## 3.3 Note

- `time_sec: float`
- `note_type: str` (`don` / `kat` / `big_don` / `big_kat`)
- `hit: bool`

## 4. `.tja` パース戦略

MVP では一般的な 1 譜面の再生に必要な範囲に限定。

1. ヘッダ行を `key:value` 形式で取得
2. `COURSE` は最初に見つかった対象難易度のみ採用（初期は `Oni` 優先）
3. `#START` 以降を小節単位で読み込む
4. 文字列ノーツ（`0-9`）を拍位置へ展開
5. `#BPMCHANGE` / `#MEASURE` を現在状態へ反映

未対応命令は `logger.warning` で通知して継続。

## 5. 判定設計

入力キーを `don` / `kat` に分類し、最も近い未ヒットノーツと比較。

- Perfect: `|delta| <= 0.035s`
- Good: `|delta| <= 0.080s`
- Miss: 上記以外

Miss は「入力時ミス」と「通過ミス（ノーツ見逃し）」を区別して集計。

## 6. スコア設計（簡易）

- Perfect: `300`
- Good: `100`
- Miss: `0`
- コンボ継続: Perfect/Good
- コンボ切れ: Miss

最終表示:

- `score`
- `max_combo`
- `perfect_count`
- `good_count`
- `miss_count`

## 7. 入力設計

- `F`, `J` -> don
- `D`, `K` -> kat

`pygame.KEYDOWN` のみ利用。キーリピートは無効化または無視。

## 8. ネットワーク/キャッシュ設計

## 8.1 取得フロー

1. `index_url` から `index.json` 取得
2. JSON を検証（必須キー存在）
3. `data/cache/index.json` へ保存
4. 選曲時に必要なら譜面/音源をダウンロード

## 8.2 キャッシュ戦略

- 曲 ID ごとに `data/cache/songs/<id>/` を作成
- 既存ファイルがある場合は再利用
- 失敗時は部分ファイルを削除

## 8.3 エラー時挙動

- index 取得失敗: 直近キャッシュを使って起動継続
- キャッシュなしで失敗: エラーダイアログ相当の画面表示

## 9. シーン遷移

- 起動 -> `SongSelectScene`
- 曲決定 -> `GameplayScene`
- 曲終了/中断 -> `ResultScene`
- 戻る操作 -> `SongSelectScene`

すべて `App.change_scene(next_scene)` で統一。

## 10. 失敗しやすい点と対策

- `.tja` 方言差異
  - 対策: 厳密パースより寛容パース（可能な限り続行）
- 音声同期ズレ
  - 対策: `OFFSET` 適用、将来的に global offset 設定
- Linux 音声環境差
  - 対策: 起動時に mixer 初期化失敗を検知し、わかりやすく通知

## 11. テスト方針

自動テストはコアロジック優先:

- `tja_parser.py` の単体テスト（サンプル `.tja`）
- `judge.py` の境界値テスト（閾値付近）
- `cache.py` の保存/読込テスト

GUI は手動確認中心:

- 起動
- 選曲
- プレイ開始
- リザルト表示

## 12. 実装優先順位

1. 画面遷移と最低限の描画
2. `.tja` 読み込み
3. 判定/スコア
4. index 取得
5. エラー処理とログ

## 13. 将来の分割方針

機能肥大化したら次を分離:

- `chart` パッケージ（パースとモデル）
- `gameplay` パッケージ（判定/スコア/入力）
- `infra` パッケージ（HTTP/キャッシュ/設定）

ただし、MVP では過剰分割しない。
