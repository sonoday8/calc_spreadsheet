# calc_spreadsheet_php

`calc_spreadsheet` を PHP から呼ぶ [ext-php-rs](https://github.com/davidcole1340/ext-php-rs) 拡張です。

- **このクレート:** セル／置換マップ／閾値の型変換と受け渡し
- **本体クレート:** 計算とプレースホルダ置換（`src/replace.rs`）

リポジトリ内の場所は `ext-php/` です。置換ルールの詳細はルート [`README.md`](../README.md) の「プレースホルダ置換」を参照してください。

## 関数

```php
calc_spreadsheet(
    array $cells,
    array $replacements = [],
    ?int $min_layer_width = null,
    ?int $min_layer_work = null
): array
```

- `$cells` — セル名 => 式またはリテラル（例: `'A1' => '=B1*2'`）。**数式は `=` 始まり**（Excel 寄せ。`=` 無しの `SUM(1)` などはテキスト）。本体クレートの全計算 API と同じ規則
- `$replacements` — `'__NAME__' => 'Alice'` / `'__RATE__' => 10`。省略可
- 第3・第4引数 — 並列適応の閾値。省略時はエンジン既定値

`$cells` だけの既存呼び出しはそのまま使えます（置換が空でも Excel 寄せの前処理は走ります）。閾値だけ位置指定する場合は第2引数に `[]` を渡します。不正な置換キー（`__[A-Z0-9]+__` 以外）は無視され、`E_USER_WARNING` が出ます（計算は続行）。規則の詳細はルート README の「プレースホルダ置換」を参照してください。

```php
$result = calc_spreadsheet($cells);
$result = calc_spreadsheet($cells, ['__NAME__' => 'Alice', '__RATE__' => 10]);
$result = calc_spreadsheet($cells, [], $min_layer_width, $min_layer_work);
```

## ビルドと実行

リポジトリルートから（PHP 開発ヘッダ / `php-config` が必要。Windows では nightly + `abi_vectorcall`）:

```bash
cargo test                              # 本体（置換テスト含む）
cargo build -p calc_spreadsheet_php --release
php -d extension=./target/release/libcalc_spreadsheet_php.so ext-php/examples/test.php
```

`php.ini` に載せる場合:

```ini
extension=calc_spreadsheet_php
; または絶対パス
; extension=/absolute/path/to/libcalc_spreadsheet_php.so
```
