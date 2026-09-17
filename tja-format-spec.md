# TJA Format — Parser開発ドキュメント

> **参考元:**
> - [KatieFrogs / TJA-format.mediawiki (Gist)](https://gist.github.com/KatieFrogs/e000f406bbc70a12f3c34a07303eec8b)
> - [Better-taiko-web / TJA-format.mediawiki (GitHub)](https://github.com/269Seahorse/Better-taiko-web/blob/master/TJA-format.mediawiki)
> - [TJAPlayer3 仕様ドキュメント](https://iepiweidieng.github.io/TJAPlayer3/tja/)

---

## 1. ファイル概要

| 項目 | 詳細 |
|------|------|
| 拡張子 | `.tja` |
| 文字コード | UTF-8 (BOM付き) または Shift-JIS |
| 改行コード | `LF` または `CR LF` |
| コメント | `//` で始まる行末まで |
| 構成 | グローバルメタデータ + 1〜複数のコースセクション |

TJAファイルは**プレーンテキスト**で，グローバルなメタデータ（曲情報）と，難易度ごとの譜面セクション（コース）から構成される。

```
TITLE:さいたま2000
BPM:150
WAVE:saitama2000.ogg
OFFSET:-2.077

COURSE:Oni
LEVEL:9
BALLOON:6,15
#START
1020,
5000,
...
#END
```

---

## 2. グローバルメタデータ（ファイル先頭）

`KEY:VALUE` の形式。コロン `:` の後にオプションでスペース可。

### 2.1 必須・重要ヘッダー

| ヘッダー | 型 | デフォルト | 説明 |
|----------|----|-----------|------|
| `TITLE:` | string | — | 曲名 |
| `BPM:` | float | `120` | 基本BPM。省略時は120 |
| `WAVE:` | string | — | 音声ファイルパス（同ディレクトリ推奨） |
| `OFFSET:` | float | `0` | 譜面オフセット（秒）。負→ノーツ遅延，正→前倒し |

### 2.2 任意ヘッダー（表示系）

| ヘッダー | 型 | 説明 |
|----------|----|------|
| `SUBTITLE:` | string | サブタイトル。先頭 `--` で結果画面非表示，`++` で表示 |
| `TITLEEN:` / `TITLEJA:` / `TITLECN:` / `TITLETW:` / `TITLEKO:` | string | 多言語タイトル |
| `SUBTITLEEN:` など | string | 多言語サブタイトル |
| `DEMOSTART:` | float | 曲選択画面でのプレビュー開始位置（秒） |
| `GENRE:` | string | ジャンル（例：`J-POP`, `アニメ`, `ゲームミュージック`） |
| `MAKER:` | string | 譜面制作者名。`<URL>` を末尾に追加可 |
| `LYRICS:` | string | 歌詞VTTファイルのパス |

### 2.3 任意ヘッダー（ゲームプレイ系）

| ヘッダー | 型 | デフォルト | 説明 |
|----------|----|-----------|------|
| `SCOREMODE:` | int (0/1/2) | `1` | スコア計算方式 |
| `SONGVOL:` | int | `100` | 音楽ボリューム (%) |
| `SEVOL:` | int | `100` | 効果音ボリューム (%) |
| `LIFE:` | int | `0` | ミス許容回数（0=無制限） |
| `HEADSCROLL:` | float | `1.0` | スクロール速度の基準倍率 |
| `BGIMAGE:` | string | — | 背景画像ファイル |
| `BGMOVIE:` | string | — | 背景動画ファイル |
| `MOVIEOFFSET:` | float | — | 動画開始オフセット（秒，非負） |
| `SIDE:` | string/int | `Both` | 表示モード: `Normal`/`1`, `Ex`/`2`, `Both`/`3` |
| `GAME:` | string | `Taiko` | `Taiko` / `Jube`（Jubeは自動演奏） |

#### SCOREMODEの詳細

```
SCOREMODE:0  -- AC1〜7世代
  < 200コンボ: INIT (1000) pt/note
  >= 200コンボ: INIT + DIFF (2000) pt/note

SCOREMODE:1  -- AC8〜14世代（デフォルト）
  INIT + max(0, DIFF * floor((min(COMBO, 100) - 1) / 10))

SCOREMODE:2  -- AC0世代
  INIT + DIFF * {COMBO>=100: 8, >=50: 4, >=30: 2, >=10: 1, else: 0}
```

---

## 3. コースメタデータ

各コース（難易度）の前に記述するヘッダー群。前のコースの値を引き継ぐ場合は省略可。

| ヘッダー | 型 | デフォルト | 説明 |
|----------|----|-----------|------|
| `COURSE:` | string/int | `Oni` | 難易度（下表参照） |
| `LEVEL:` | int (1〜10) | — | 星の数 |
| `BALLOON:` | int[], CSV | — | バルーンノーツの打数リスト |
| `SCOREINIT:` | int | — | スコア計算のINIT値 |
| `SCOREDIFF:` | int | — | スコア計算のDIFF値 |
| `BALLOONNOR:` | int[], CSV | — | 分岐「普通」用バルーン打数 |
| `BALLOONEXP:` | int[], CSV | — | 分岐「玄人」用バルーン打数 |
| `BALLOONMAS:` | int[], CSV | — | 分岐「達人」用バルーン打数 |
| `STYLE:` | string/int | `Single` | `Single`/`1` or `Double`/`2`（2P譜面） |

### COURSEの値

| 文字列 | 数値 | 説明 |
|--------|------|------|
| `Easy` | `0` | かんたん |
| `Normal` | `1` | ふつう |
| `Hard` | `2` | むずかしい |
| `Oni` | `3` | おに |
| `Edit` / `Ura` | `4` | 裏おに（隠し難易度） |
| `Tower` | `5` | とうもろこし（ドン！）モード |
| `Dan` | `6` | 段位道場モード |

---

## 4. 譜面セクション

コースメタデータの後，`#START` と `#END` で囲まれた領域が実際の譜面。

```
#START
1020,
0050,
#END
```

### 4.1 ノーツ定義

各文字が1つのノーツを表す。数字を並べて1小節（measure）を構成し，末尾をカンマ `,` で区切る。

| 文字 | ノーツ種別 |
|------|-----------|
| `0` | 空白（ノーツなし） |
| `1` | ドン（小）|
| `2` | カッ（小）|
| `3` | ドン（大）|
| `4` | カッ（大）|
| `5` | ドラムロール（開始）|
| `6` | ドラムロール・大（開始）|
| `7` | バルーンノーツ（開始）|
| `8` | ロール/バルーン 終了 |
| `9` | 九周（Kusudama）ノーツ（開始，`8`で終了） |
| `A` | 両手ドン（2P用）|
| `B` | 両手カッ（2P用）|
| `F` | ADLIBノーツ（隠しノーツ，コンボ加算，BAD判定なし）|

### 4.2 小節の仕組み

- 小節は `,` + 改行で区切る
- 1小節内の文字数がそのまま分割数（タイミング）を決める
- 例：`1020,` → 4分割（4分音符単位），`10002000,` → 8分割（8分音符単位）
- 以下はすべて同じタイミング：`12,` / `1020,` / `10002000,`

```
// 4拍子でドンとカッを交互に打つ例
1010,   // ドン・空・ドン・空
2020,   // カッ・空・カッ・空
```

---

## 5. コマンド（譜面内ディレクティブ）

`#COMMAND` または `#COMMAND value` の形式で譜面セクション内に記述。

### 5.1 基本コマンド

| コマンド | 説明 |
|----------|------|
| `#START` | 譜面開始（`P1`/`P2` を引数に取ることで2P譜面の側を指定） |
| `#END` | 譜面終了 |
| `#MEASURE n/m` | 拍子変更（例：`#MEASURE 3/4`） |
| `#BPMCHANGE bpm` | BPM変更 |
| `#SCROLL speed` | スクロール速度変更（倍率，例：`#SCROLL 1.5`） |
| `#DELAY sec` | 指定秒数だけ次ノーツのタイミングをずらす |
| `#GOGOSTART` | ゴーゴータイム開始 |
| `#GOGOEND` | ゴーゴータイム終了 |
| `#BARLINEOFF` | 小節線を非表示にする |
| `#BARLINEON` | 小節線を表示に戻す |
| `#LYRIC text` | 歌詞テキスト（`LYRICS:` ヘッダーが優先） |

### 5.2 分岐コマンド（BRANCH）

| コマンド | 説明 |
|----------|------|
| `#BRANCHSTART type,exp,mas` | 分岐開始。`type` は `r`（ロール数）/ `p`（精度%）/ `s`（スコア）。判定は分岐の1小節前の小節線で行う |
| `#N` | 「普通」分岐ラベル |
| `#E` | 「玄人」分岐ラベル |
| `#M` | 「達人」分岐ラベル |
| `#BRANCHEND` | 分岐終了，全分岐を合流 |
| `#SECTION` | 分岐判定用の成績（ロール数・精度・スコア）をこの時点でリセット |
| `#LEVELHOLD` | 分岐判定を保持（以降の #BRANCHSTART では再評価しない） |

```
#BRANCHSTART r,10,20
// 直前のドラムロール数が 10未満→普通, 10〜19→玄人, 20以上→達人
#N
1010,
#E
1111,
#M
1111,1111,
#BRANCHEND
```

### 5.3 段位道場コマンド（COURSE:Danのみ）

| コマンド | 説明 |
|----------|------|
| `#SUDDEN n` | n個目のゴーゴー開始時に暗転エフェクト |
| `#JPOSSCROLL dur, dist, side` | ゲージ位置スクロール |

---

## 6. ファイル全体の構造（BNF風）

```
tja_file      = global_header* course_block+

global_header = key ":" value newline

course_block  = course_header* notation_block

course_header = key ":" value newline

notation_block = "#START" [" P1"|" P2"] newline
                 (measure | command)*
                 "#END" newline

measure       = note_chars "," newline
note_chars    = ("0"|"1"|"2"|"3"|"4"|"5"|"6"|"7"|"8"|"9"|"A"|"B"|"F")*

command       = "#" COMMAND_NAME [" " args] newline
```

---

## 7. Parserの実装指針

### 7.1 処理フロー

```
1. ファイル読み込み（Shift-JIS または UTF-8 BOM付き）
2. 行単位で処理
   a. 空行・コメント（//）をスキップ
   b. KEY:VALUE 形式 → グローバルまたはコースヘッダーとして登録
   c. #START → 譜面パースモードに移行
   d. #END   → 譜面パースモード終了，コースをリストに追加
   e. 譜面パースモード中:
      - #COMMAND [args] → コマンドとして処理
      - それ以外 → カンマで区切ってmeasure（ノーツ列）として処理
3. 全コース分繰り返し
```

### 7.2 タイミング計算

ノーツの発音タイミング（秒）は下記の手順で算出する。

```python
# beat_duration: 1拍の長さ（秒）
beat_duration = 60.0 / current_bpm

# measure_duration: 1小節の長さ（秒）
measure_duration = beat_duration * (numerator / denominator) * 4
# ※ 4/4拍子なら beat_duration * 4

# 1小節に n 個のノーツがある場合，各ノーツ間隔
note_interval = measure_duration / n

# i番目のノーツの絶対時刻（OFFSETを考慮）
note_time = current_time + note_interval * i - OFFSET
```

- `#BPMCHANGE` はそのノーツのタイミングから次のBPMを適用する
- `#SCROLL` はタイミングには影響せず，視覚的なスクロール速度のみ変更
- `#DELAY` は次の小節の開始時刻をずらす

### 7.3 データモデル（例）

```python
@dataclass
class TJASong:
    title: str
    bpm: float
    wave: str
    offset: float
    courses: list[TJACourse]

@dataclass
class TJACourse:
    course: int          # 0=Easy ... 4=Edit
    level: int
    balloon: list[int]
    score_init: int
    score_diff: int
    measures: list[TJAMeasure]

@dataclass
class TJAMeasure:
    notes: list[TJANote]
    bpm: float           # この小節開始時のBPM
    scroll: float        # スクロール速度倍率
    is_gogotime: bool

@dataclass
class TJANote:
    note_type: int       # 0〜9, A, B, F
    time: float          # 発音時刻（秒）
    duration: float      # ロール/バルーンの場合の持続時間（秒）
```

### 7.4 実装上の注意点

- **文字コード検出:** BOMがあればUTF-8，なければShift-JIS と判断するのが安全。`chardet` などのライブラリも有効
- **ロール/バルーンの終端:** `5`/`6`/`7`/`9` は対応する `8` が来るまで継続。スタックで管理する
- **BALLOON配列:** `BALLOON:6,15,3` は譜面内の `7` の出現順に対応。分岐がある場合は `BALLOONNOR:`/`BALLOONEXP:`/`BALLOONMAS:` を参照
- **BPM変更のタイミング:** `#BPMCHANGE` は直後の小節から反映ではなく，**コマンドが記述された位置**（現在の小節内の処理済みノーツの次）から即時反映
- **空小節:** ノーツ0個の `,` も有効（タイミング送りとして機能）
- **2P譜面:** `#START P1` / `#START P2` で同一コース内に2人分の譜面を記述可能

---

## 8. サンプルTJAファイル

```
TITLE:サンプル曲
BPM:140
WAVE:sample.ogg
OFFSET:-1.5
DEMOSTART:30.0
GENRE:J-POP

COURSE:Oni
LEVEL:7
BALLOON:10,20
SCOREINIT:500
SCOREDIFF:100

#START
// ゴーゴータイムなし普通パート
1020,
2010,
#GOGOSTART
// ゴーゴータイム
1111,
2222,
#GOGOEND
// バルーン
7000,8000,
// ドラムロール
5000,8010,
#END

COURSE:Hard
LEVEL:5

#START
1010,
2020,
#END
```

---

## 9. 参考リンク

| リソース | URL |
|----------|-----|
| KatieFrogs TJA仕様 (Gist) | https://gist.github.com/KatieFrogs/e000f406bbc70a12f3c34a07303eec8b |
| Better-taiko-web 仕様 (GitHub) | https://github.com/269Seahorse/Better-taiko-web/blob/master/TJA-format.mediawiki |
| TJAPlayer3 詳細仕様 | https://iepiweidieng.github.io/TJAPlayer3/tja/ |
| Project OutFox TJA対応表 | https://outfox.wiki/en/dev/mode-support/tja-support |
| tja.js (JavaScript実装) | https://github.com/jozsefsallai/tja-js |
