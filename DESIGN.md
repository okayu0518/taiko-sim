# DESIGN

Windows / Linux向けのRust製太鼓系リズムゲームシミュレータ。KISS方針で、macroquadによる表示・入力とrodioによる音声再生を採用。

## 実装構成

- `src/main.rs`: リソースルートと設定の読み込み、起動
- `src/game.rs`: 選曲・プレイ・結果画面、入力、音声と描画
- `src/gameplay.rs`: 描画と独立した判定・連打・分岐・スコア
- `src/judge.rs`: 良（±25ms） / 可（±75ms） / 不可
- `src/tja_parser.rs`: TJAの解析
- `src/repository.rs`: ローカル譜面とindex.jsonの検出
- `src/models.rs`: 設定、譜面、スコア
- `src/effects.rs`: エフェクト

## ファイルの配置

配布版は実行ファイルが置かれたフォルダの `assets/`、`data/`、`config.json` を利用する。これらがない開発環境ではカレントディレクトリから読む。配布ZIPはexe、assets、config.jsonと空のsongsディレクトリにREADME.txtを含む。

## 入力

4つのバインドキーを個別チェックする。同時フレームの2キー入力も個別に処理するが、macroquadのフレーム単位の入力では同一キーの高速な2打鍵は区別できない。

## 音声

BGMはrodio Sink、SEは事前デコードした音源をplay_rawで再生する。SEは元のサンプルレートとチャンネル数を保持する。入力からSEソースの最初の読み出しまでの時間は実際に耳に届く遅延ではない。

## 制約と課題

BGMと判定は現在別のクロックに依存するため、長時間の同期ズレやOS固有の出力遅延は今後検証が必要。Windows実機の音声・表示・入力の確認も必要。

## CI

Ubuntuでビルド・テスト、Windows MSVCでテストとreleaseビルドを実施し、WindowsポータブルZIPをArtifactsにアップロードする。

## UI操作（D/F/J/K）

- 選曲: D/Kで前後の曲、Jで決定（0曲の場合は再読み込み）、Fで終了。
- プレイ: D/KでKat、F/JでDon。F+Jの1.5秒長押しでリザルトへ。
- リザルト: Fで選曲に戻る、Jで再プレイ。
- ゲームプレイのキー割り当てはconfig.jsonで変更可能。メニュー操作は物理D/F/J/K固定。
- 従来の矢印・Enter・Escapeショートカットは互換性のため残す。

`data/songs/README.md` をリポジトリに保持し、譜面・音源はGit管理対象外とする。ゲーム初回起動でも `data/songs/` を作成する。
