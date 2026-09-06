# calc_spreadsheet

Excel / PhpSpreadsheet に寄せた計算式を評価する Rust ライブラリです。数値・文字列セル、依存解決、適応的並列評価、動的配列（スピル）に対応しています。

License: MIT（[`LICENSE`](LICENSE)）

## 使い方

```rust
use calc_spreadsheet::{calculate_spreadsheet, CellValue};

let cells = [
    ("A1", "2"),
    ("A2", "3"),
    ("B1", "10"),
    ("B2", "20"),
    ("C1", "=SUM(A1:A2*B1:B2)"),
    ("D1", "=SEQUENCE(3)"),
];

let values = calculate_spreadsheet(&cells)?;
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
| `calculate_spreadsheet(cells)` | デフォルト閾値で層ごとに seq / rayon を自動選択 |
| `calculate_spreadsheet_with_thresholds(cells, thresholds)` | 幅・仕事量の閾値を指定して適応並列 |
| `ParallelThresholds` | `min_layer_width` / `min_layer_work`（`Default` は 4096 / 327680） |
| `calculate_spreadsheet_with_parallel(cells, parallel)` | 層内並列を強制 ON/OFF（計測用・`doc(hidden)`） |
| `CellValue` | `Number(f64)` / `Text(String)` |
| `SpreadsheetError` | 循環参照・不正数式・ゼロ除算・`#NUM!` / `#VALUE!` / `#SPILL!` / `#CALC!` |

入力は `&[(&str, &str)]`（セル名 → 式またはリテラル）。戻りは評価後の全セル（スピル先を含む）の `HashMap` です。セル名は大文字小文字を区別しません。

## 数式・配列

- **演算:** `+ - * /`、比較 `> < >= <= = == <> !=`
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

- `FILTER(array, include, [if_empty])` の `if_empty` は **空のときだけ**評価（lazy）。非空パスでは参照も依存に入れません
- 静的に空／非空が分からないとき、`if_empty` の参照は集めますが、この `FILTER` 自身の静的スピル領域内のセルは除外します（自己スピル読みによる偽循環を防ぐ）
- **soft-skip:** `include` などがスピル先を読み、かつその `FILTER` アンカーが読者に依存して循環になる場合、依存辺を張らずに評価し、その後アンカーと推移的依存を再評価して整合させます（最大 2 スイープ）
- soft-skip の対象は **スピル源が `FILTER` のときだけ**（トップレベル、または `IFERROR` / `IF` / `IFS` / `SWITCH` / `LET` 経由。定数刈り込み後の死腕は対象外）。`SEQUENCE` などによる同様の循環は従来どおり `CircularReference` です。`SEQUENCE(..)+FILTER(..)` や `-FILTER(..)` のような演算子結合も soft-skip しません

### `LET`

`LET(name1, value1, …, calculation)` でローカル束縛できます。名前は識別子または文字列リテラル。評価時にスコープされ、スピル形状・soft-skip 判定では計算式へ展開します。参照解析は束縛を宣言順に集め、すでに束縛した名前と計算式中の束縛名はシート参照にしません（値式のシート参照は依存に残します）。

### 対応関数

| 分類 | 関数 |
|---|---|
| 集計 | `SUM`, `AVERAGE`, `MIN`, `MAX`, `PRODUCT`, `COUNT`, `COUNTA` |
| 数学 | `ABS`, `INT`, `SQRT`, `POWER`, `MOD`, `ROUND`, `ROUNDUP`, `ROUNDDOWN`（`SQRT` の負数は `#NUM!`） |
| 論理 | `IF`, `IFS`, `SWITCH`, `IFERROR`, `AND`, `OR`, `NOT`, `LET` |
| 日付 | `DATE`, `DATEVALUE`, `YEAR`, `MONTH`, `DAY`, `DAYS`, `DATEDIF` |
| 動的配列 | `SEQUENCE`, `UNIQUE`, `SORT`, `FILTER` |

動的配列は現状 **数値配列のみ** です。`FILTER` の `include` は行ベクトルまたは列ベクトルを想定しています。

## 並列の適応判定

各評価層について次を満たすときだけ rayon を使います（`examples/bench_threshold` の実測交差点ベース）。

- 層のセル数 `>= min_layer_width`（デフォルト **4096**）
- 層内の仕事量 `>= min_layer_work`（デフォルト **327680**＝セル参照の出現回数。A1 範囲は展開後のセル数）

※ AST 化後の再計測（2026-09-05）では、以前の 1024×80 は speedup が 1 未満になり、初めて安定して ≥1.1 になったのは **4096×80** でした。

### 閾値の決め方（キャリブレーション）

対象マシンで `--release` 実行し、speedup ≥ 1.1 になる最初の格子点を推奨値として出します。

```bash
cargo run --release --example bench_threshold
```

出力末尾の `min_layer_width` / `min_layer_work` を次に渡します。

- Rust: `calculate_spreadsheet_with_thresholds(cells, ParallelThresholds { … })`
- PHP: `calc_spreadsheet($cells, $min_layer_width, $min_layer_work)`

**ホスト依存:** デフォルトはキャリブレーションしたマシン向けです。別 CPU では必ず再計測してください。

## PHP 拡張 (`ext-php/`)

クレート名は `calc_spreadsheet_php`（cdylib）。ビルドには PHP 開発ヘッダと [ext-php-rs](https://github.com/davidcole1340/ext-php-rs) が必要です。

```bash
cargo build -p calc_spreadsheet_php --release
```

```php
$result = calc_spreadsheet($cells);
// 閾値を上書きする場合:
$result = calc_spreadsheet($cells, $min_layer_width, $min_layer_work);
```

混合シートの簡易計測例: `ext-php/examples/test.php`

## 非対応（スコープ外）

- シート参照（`Sheet1!A1`）
- `INDIRECT` / 構造化参照
- テキスト配列の動的配列
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
| `value.rs` / `error.rs` | `CellValue` / `SpreadsheetError` |
| `tests.rs` | 統合テスト |
| `main.rs` | 日付関数の簡単なデモバイナリ |
| `ext-php/` | PHP 拡張（`calc_spreadsheet_php`） |

## ベンチ

```bash
# 混合シートでの seq / forced-par / adaptive 比較
cargo run --release --example bench_parallel

# 幅 × refs/cell グリッドから推奨閾値を出す（ホスト依存）
cargo run --release --example bench_threshold
```
