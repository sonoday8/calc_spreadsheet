# calc_spreadsheet

Excel / PhpSpreadsheet に寄せた計算式を評価する Rust ライブラリです。数値・文字列セル、依存解決、適応的並列評価、動的配列（スピル）、プレースホルダ置換に対応しています。

License: MIT（[`LICENSE`](LICENSE)）

## 使い方

```rust
use calc_spreadsheet::{calculate_spreadsheet, CalculateOptions, CellValue};

let cells = [
    ("A1", "2"),
    ("A2", "3"),
    ("B1", "10"),
    ("B2", "20"),
    ("C1", "=SUM(A1:A2*B1:B2)"),
    ("D1", "=SEQUENCE(3)"),
];

let outcome = calculate_spreadsheet(&cells, CalculateOptions::default())?;
let values = outcome.values;
assert_eq!(values["C1"], CellValue::Number(80.0));
assert_eq!(values["D1"], CellValue::Number(1.0));
assert_eq!(values["D2"], CellValue::Number(2.0)); // スピル
assert_eq!(values["D3"], CellValue::Number(3.0));
```

```bash
cargo run          # src/main.rs の日付デモ
cargo test
```

## 公開 API

| 項目 | 内容 |
|---|---|
| `calculate_spreadsheet(cells, options)` | 唯一の計算入口。置換・閾値は `CalculateOptions` で任意指定 |
| `CalculateOptions` | `replacements` / `thresholds`（どちらも `None` で既定） |
| `SpreadsheetOutcome` | `values` と `ignored_replacement_keys`（不正キー警告用） |
| `ReplacementValue` | `from_i64` / `from_f64` / `from_text` のみ（内部表現は非公開） |
| `format_number` | 数値の正規文字列化（PHP 拡張などが利用） |
| `ParallelThresholds` | `min_layer_width` / `min_layer_work`（`Default` は 4096 / 327680） |
| `CellValue` | `Number(f64)` / `Text(String)` |
| `SpreadsheetError` | 循環参照・不正数式・ゼロ除算・`#NUM!` / `#VALUE!` / `#SPILL!` / `#CALC!` |

入力は `&[(&str, &str)]`（セル名 → 式またはリテラル）。戻りは `SpreadsheetOutcome`（`values` に評価後の全セル。スピル先を含む）。セル名は大文字小文字を区別しません。

**Excel 寄せの入力規則（すべての計算 API 共通）:**

- **数式は `=` で始める**（`SUM(1)` のように `=` 無しは式にせずテキスト）
- **数値リテラル**（`10` / `1.5`）はそのまま数値
- **裸の非数値テキスト**は文字列セルとして扱う

## 数式・配列

- **演算:** `+ - * /`、文字列連結 `&`（算術より低く比較より高い）、比較 `> < >= <= = == <> !=`
- **範囲:** A1 形式（例: `A1:A3`）。AST 上は端点のみ保持し、参照解析・集計時にストリーム展開
- **要素演算:** 同形配列同士、スカラー↔配列、Excel 風の **1×N ⊗ M×1** ブロードキャスト。それ以外の形状不一致は `#VALUE!`
- **スピル:** 配列結果はアンカーから矩形に配置し、戻り値にスピル先を含む。入力セルや他式の値と衝突すると `#SPILL!`。書き込み前にフットプリント全体を検証するため、失敗時に半端なスピルは残しません。同一アンカーの再評価では旧スピル領域を消してから書き直します
- **スピル参照:** `A1#` でアンカーの動的配列全体を参照（例: `SUM(A1#)`）
- **暗黙交差:** `@A1:A3` で評価セルと同じ行／列の値を取得（CSE の `{}` は不要）
- **上限:** 展開・配列サイズが 100,000 セルを超えると `#NUM!`
- **空白セル:** 算術・参照・配列要素では Excel 同様に **0**。`SUM` / `AVERAGE` / `PRODUCT` / `MIN` / `MAX` / `COUNT` は空白とテキストをスキップ（すべて空白の `MIN`/`MAX` は **0**）。`COUNTA` はテキストを数え、空白は数えない
- **集計と空配列:** 空の `FILTER` / `UNIQUE` / `SEQUENCE(0,…)` は `#CALC!`（`FILTER` に `if_empty` がある場合はそちら）
- **条件分岐の静的刈り込み:** 定数に畳める `IF` / `IFS` / `SWITCH` は未使用腕の参照・スピル形状を落とし、偽の循環や余分なスピル辺を避けます。条件が不明なときは両腕を合成します

### `FILTER` と soft-skip

- `if_empty` は空のときだけ評価（lazy）
- スピル先を読む `FILTER` が自己依存になる循環は、依存辺を張らず後から再評価して整合（最大 2 スイープ）。対象は `FILTER` スピル源のみ（`IFERROR` / `IF` / `IFS` / `SWITCH` / `LET` 経由可）。`SEQUENCE` や演算子で包んだ `FILTER` は soft-skip しない

### `LET`

`LET(name1, value1, …, calculation)` でローカル束縛。識別子または文字列リテラル。束縛名はシート参照にせず、値式のシート参照だけ依存に残します。

### 対応関数

| 分類 | 関数 |
|---|---|
| 集計 | `SUM`, `AVERAGE`, `MIN`, `MAX`, `PRODUCT`, `COUNT`, `COUNTA` |
| 数学 | `ABS`, `INT`, `SQRT`, `POWER`, `MOD`, `ROUND`, `ROUNDUP`, `ROUNDDOWN`（`SQRT` の負数は `#NUM!`） |
| 論理 | `IF`, `IFS`, `SWITCH`, `IFERROR`, `AND`, `OR`, `NOT`, `LET` |
| 日付 | `DATE`, `DATEVALUE`, `YEAR`, `MONTH`, `DAY`, `DAYS`, `DATEDIF` |
| 動的配列 | `SEQUENCE`, `UNIQUE`, `SORT`, `FILTER` |

動的配列は現状 **数値配列のみ** です。`FILTER` の `include` は行ベクトルまたは列ベクトルを想定しています。

## プレースホルダ置換

実装は `src/replace.rs`。評価前にセル文字列へ `__[A-Z0-9]+__` を埋め込みます。値は文字列か数字のみ（式は入れない）。`ReplacementValue` は `from_i64` / `from_f64` / `from_text` でのみ生成します。セル前処理（`prepare_*`）はクレート内専用です。

入力規則（`=` 必須など）は上の「公開 API」と同じで、置換マップが空でも前処理は走ります。

| セルの書き方 | 意味 |
|---|---|
| `A1` / `=A1` | セル参照 |
| `__A1__` / `__NAME__` | プレースホルダ |

- `NAME` や `__name__`、`_NAME_`、`__USER_NAME__`（内側に `_`）は置換しない（**無視し、呼び出し側へ報告**。Rust は `ignored_replacement_keys`、PHP は `E_USER_WARNING`。計算は続行）
- マップに無い `__FOO__` は残す。テキストセルなら文字として返す（置換し忘れが見える）。**式に残すと欠落セル同様に 0 扱い**（`=Z99` と同じ）
- `__NAME__` が `__NAMESPACE__` の一部になることはない
- 置換値の中の `__FOO__` は再展開しない
- `=` で始まる置換値は式にせずテキストとして入れる
- **Excel 文字列リテラル（`"..."`、`""` エスケープ）の内側は置換しない**。クォートの外だけ置換する（例: `="Hi "&__NAME__` → `="Hi "&"Alice"` → 結果 `Hi Alice`）
- **隣接プレースホルダは区切りを書く**（`=__A__&__B__` や `=__A__+__B__`）。`=__A____B__` のように繋ぐと、テキストは `="X""Y"`（中に `"` が入る1文字列）になり、数値どうしは桁がくっつく

```rust
use calc_spreadsheet::{
    calculate_spreadsheet, CalculateOptions, ReplacementValue,
};
use std::collections::HashMap;

let cells = [
    ("A1", "__NAME__"),
    ("B1", "NAME"),
    ("C1", "__UNKNOWN__"),
    ("D1", "=__RATE__*2"),
    ("E1", "=F1+1"),
    ("F1", "=__A1__"),
];
let mut replacements = HashMap::new();
replacements.insert("__NAME__".into(), ReplacementValue::from_text("Alice"));
replacements.insert("__RATE__".into(), ReplacementValue::from_i64(10));
replacements.insert("__A1__".into(), ReplacementValue::from_i64(7));

let outcome = calculate_spreadsheet(
    &cells,
    CalculateOptions {
        replacements: Some(&replacements),
        ..Default::default()
    },
)?;
let values = outcome.values;
// outcome.ignored_replacement_keys に不正キー（あれば）
// A1 = Alice, B1 = NAME, C1 = __UNKNOWN__（テキスト）
// D1 = 20, E1 = 8, F1 = 7
```

Excel のシート上限（xlsx）は列 A〜XFD、行 1〜1048576 です。セル番地に `__` は使いません。

## 並列の適応判定

各評価層について次を満たすときだけ rayon を使います。

- 層のセル数 `>= min_layer_width`（デフォルト **4096**）
- 層内の仕事量 `>= min_layer_work`（デフォルト **327680**＝セル参照の出現回数。A1 範囲は展開後のセル数）

※ デフォルトは speedup ≥ 1.1 になった格子点（幅 4096 × refs/cell 80）基準です。

### 閾値の決め方（キャリブレーション）

対象マシンで `--release` 実行し、speedup ≥ 1.1 になる最初の格子点を推奨値として出します。

```bash
cargo run --release --example bench_threshold
```

出力末尾の `min_layer_width` / `min_layer_work` を次に渡します。

- Rust: `calculate_spreadsheet(cells, CalculateOptions { thresholds: Some(ParallelThresholds { … }), ..Default::default() })`
- PHP: `calc_spreadsheet($cells, [], $min_layer_width, $min_layer_work)`（第2引数は置換マップ）

**ホスト依存:** デフォルトはキャリブレーションしたマシン向けです。別 CPU では必ず再計測してください。

## PHP 拡張 (`ext-php/`)

クレート名は `calc_spreadsheet_php`（cdylib）。型変換と引数受け渡しのみで、計算・置換は本体に委譲します。ビルドには PHP 開発ヘッダと [ext-php-rs](https://github.com/davidcole1340/ext-php-rs) が必要です。詳細なビルド手順は [`ext-php/README.md`](ext-php/README.md)。

```bash
cargo build -p calc_spreadsheet_php --release
```

成果物（Linux 例）: `target/release/libcalc_spreadsheet_php.so` を `extension_dir` へ置くか、`php.ini` でパス指定します。

```ini
extension=calc_spreadsheet_php
```

```php
calc_spreadsheet(
    array $cells,
    array $replacements = [],
    ?int $min_layer_width = null,
    ?int $min_layer_work = null
): array

$result = calc_spreadsheet($cells);
$result = calc_spreadsheet($cells, ['__NAME__' => 'Alice', '__RATE__' => 10]);
$result = calc_spreadsheet($cells, [], $min_layer_width, $min_layer_work);
```

プレースホルダ規則は上の「プレースホルダ置換」と同じです。簡易計測例: `ext-php/examples/test.php`

## 非対応（スコープ外）

- シート参照（`Sheet1!A1`）
- `INDIRECT` / 構造化参照
- テキスト配列の動的配列（`&` はスカラー連結のみ。配列との `&` は `#VALUE!`）
- 非 A1 名セルからの幾何スピル（左上のみ採用）
- Excel 全関数・全版差・全ブロードキャスト規則の完全再現
- `LET` の Excel 全意味論（名前影の細部など）の完全再現

## ソース構成

| ファイル | 役割 |
|---|---|
| `lib.rs` | 公開 API・層評価・スピル配置・FILTER soft-skip fixup |
| `ast.rs` | 式のパース・評価・参照解析（単一 AST） |
| `dynamic_array.rs` | `SEQUENCE` / `UNIQUE` / `SORT` / `FILTER` |
| `deps.rs` | 依存グラフ・評価層分け |
| `refs.rs` | A1 範囲・仕事量・サイズ上限 |
| `spreadsheet.rs` | 評価コンテキスト（`LET` 束縛を含む） |
| `functions.rs` | 即時評価の集計・数学・論理関数 |
| `excel_date.rs` | Excel 日付シリアル・`DATEDIF` など |
| `parser.rs` | 純粋文字列リテラル判定 |
| `replace.rs` | プレースホルダ置換（`__[A-Z0-9]+__`） |
| `value.rs` / `error.rs` | `CellValue` / `SpreadsheetError` |
| `tests.rs` | 統合テスト |
| `main.rs` | 日付関数の簡単なデモバイナリ |
| `ext-php/` | PHP 拡張ラッパ（`calc_spreadsheet_php`） |

## ベンチ

```bash
# 混合シートでの seq / forced-par / adaptive 比較
cargo run --release --example bench_parallel

# 幅 × refs/cell グリッドから推奨閾値を出す（ホスト依存）
cargo run --release --example bench_threshold
```
