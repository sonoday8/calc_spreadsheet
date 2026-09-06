# calc_spreadsheet

Excel / PhpSpreadsheet 互換の計算式を評価する Rust ライブラリです。

## ソース構成

| ファイル | 役割 |
|---|---|
| `lib.rs` | 公開 API（`calculate_spreadsheet`） |
| `deps.rs` | 依存グラフ・評価層分け |
| `refs.rs` | A1 範囲展開・仕事量定数 |
| `ast.rs` | 式 AST（評価と参照解析の単一文法・1回パース） |
| `value.rs` | `CellValue` |
| `error.rs` | `SpreadsheetError` |
| `spreadsheet.rs` | 評価コンテキスト（式・キャッシュ参照） |
| `parser.rs` | AST パース入口・文字列リテラル |
| `functions.rs` | `SUM` / `ROUND` / `AND` など即時評価関数 |
| `excel_date.rs` | Excel 日付シリアル・`DATEDIF` など |
| `tests.rs` | 統合テスト |

## 公開 API

- `calculate_spreadsheet(cells)` — 層ごとに幅・仕事量を見て seq/par を自動選択
- `calculate_spreadsheet_with_parallel(cells, parallel)` — 層内 rayon を強制 ON/OFF（計測用）
- `CellValue::Number(f64)` / `CellValue::Text(String)`
- `SpreadsheetError`

## 並列の適応判定

各評価層について次を満たすときだけ rayon を使います（`examples/bench_threshold` の実測交差点ベース）。

- 層のセル数 `>= 4096`（幅）
- 層内の**仕事量** `>= 327680`（セル参照の出現回数。`A1:A3` のような A1 範囲は展開後のセル数）

A1 範囲は AST 上では端点のみ保持し、参照解析・集計時にストリーム展開します（中間の `Vec` を避けます）。集計関数引数では複数値に展開、スカラー位置の複数セル範囲は `#VALUE!`、展開セル数が 100,000 を超える範囲は `#NUM!` です。`SUM` / `AVERAGE` / `COUNT` はテキストを無視し、`COUNTA` はテキストも数えます。

※ AST 化後の再計測（2026-09-05）では、以前の 1024×80 は speedup が 1 未満になり、初めて安定して ≥1.1 になったのは **4096×80** でした。

**ホスト依存:** 閾値はキャリブレーションしたマシン向けの定数です。別 CPU / コア数では `cargo run --release --example bench_threshold` で再計測し、必要なら `WIDTH_THRESHOLD` / `WORK_THRESHOLD` を更新してください。

## 対応しているセル関数

| 分類 | 関数 |
|---|---|
| 集計 | `SUM`, `AVERAGE`, `MIN`, `MAX`, `PRODUCT`, `COUNT`, `COUNTA` |
| 数学 | `ABS`, `INT`, `SQRT`, `POWER`, `MOD`, `ROUND`, `ROUNDUP`, `ROUNDDOWN` |
| 論理 | `IF`, `IFS`, `SWITCH`, `IFERROR`, `AND`, `OR`, `NOT` |
| 日付 | `DATE`, `DATEVALUE`, `YEAR`, `MONTH`, `DAY`, `DAYS`, `DATEDIF` |

比較演算子 `>`, `<`, `>=`, `<=`, `=`, `==`, `<>`, `!=` と、A1 形式の範囲参照（例: `SUM(A1:A3)`）にも対応しています。

## 非対応（現状のスコープ外）

- シート参照（`Sheet1!A1`）
- `INDIRECT` / 構造化参照 / 動的配列のスピル
- 完全な Excel 配列演算

## ベンチ

```bash
# 混合シート（細い軽い層多数 + 重い広い層）での seq / forced-par / adaptive 比較
# 意図: forced-par は seq より遅く、adaptive は seq より速い
cargo run --release --example bench_parallel

# 幅 × refs/cell グリッドで閾値キャリブレーション
cargo run --release --example bench_threshold
```
