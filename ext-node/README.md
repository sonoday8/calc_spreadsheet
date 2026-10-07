# calc_spreadsheet (Node / TypeScript)

`calc_spreadsheet` を Node.js / TypeScript から呼ぶ [napi-rs](https://napi.rs) バインディングです。NestJS などのサーバーサイド TypeScript から利用できます。

- **このクレート:** セル／置換マップ／閾値の型変換と受け渡し
- **本体クレート:** 計算とプレースホルダ置換（`src/replace.rs`）

計算は本体クレートに委譲するため、重いシートでは大半の時間がそこにかかります。一方で JS オブジェクト ↔ Rust の変換と napi 経由の受け渡しがあるため、Rust 本体の直接呼び出しと比べると **数％程度の上乗せ**を見込んでください（PHP 拡張の FFI／HashMap 変換オーバーヘッドと同趣旨。ルート README のベンチ節も参照）。

リポジトリ内の場所は `ext-node/` です。置換ルールの詳細はルート [`README.md`](../README.md) の「プレースホルダ置換」を参照してください。

## 関数

```ts
function calcSpreadsheet(
  cells: Record<string, string | number>,
  replacements?: Record<string, string | number> | null,
  minLayerWidth?: number | null,
  minLayerWork?: number | null,
): Record<string, string | number>
```

- `cells` — セル名 => 式またはリテラル（例: `{ A1: '=B1*2' }`）。**数式は `=` 始まり**（Excel 寄せ。`=` 無しの `SUM(1)` などはテキスト）。本体クレートの全計算 API と同じ規則
- `replacements` — `{ __NAME__: 'Alice', __RATE__: 10 }`。省略可
- 第3・第4引数 — 並列適応の閾値。省略時はエンジン既定値

`cells` だけの既存呼び出しはそのまま使えます。閾値だけ指定する場合は第2引数に `{}` を渡します。不正な置換キー（`__[A-Z0-9]+__` 以外）は無視され、`console.warn` が出ます（計算は続行）。

```ts
import { calcSpreadsheet } from 'calc_spreadsheet'

const result = calcSpreadsheet(cells)
const result2 = calcSpreadsheet(cells, { __NAME__: 'Alice', __RATE__: 10 })
const result3 = calcSpreadsheet(cells, {}, minLayerWidth, minLayerWork)
```

## NestJS

特別なモジュール登録は不要です。通常の npm 依存として Service から import してください。

```ts
// app.service.ts
import { Injectable } from '@nestjs/common'
import { calcSpreadsheet } from 'calc_spreadsheet'

@Injectable()
export class AppService {
  evaluate(cells: Record<string, string | number>) {
    return calcSpreadsheet(cells)
  }
}
```

ローカル開発では:

```json
{
  "dependencies": {
    "calc_spreadsheet": "file:../calc_spreadsheet/ext-node"
  }
}
```

ネイティブアドオンは OS/arch ごとにビルドが必要です（`npm run build`）。

## ビルドと実行

Node.js 18+ と Rust toolchain が必要です。リポジトリの `ext-node/` で:

```bash
npm install
npm run build          # release ネイティブアドオン
npm test               # node:test
npm run example        # examples/test.mjs（PHP の test.php 相当）
npm run bench          # examples/bench_replace_load.mjs
```

```bash
# 個別実行
node examples/test.mjs
node examples/bench_replace_load.mjs
node --test test/*.test.mjs
```
