<?php
/**
 * Load bench matching examples/bench_replace_load.rs (large profile).
 *
 *   cargo build -p calc_spreadsheet_php --release
 *   php -d extension=./target/release/libcalc_spreadsheet_php.so \
 *       ext-php/examples/bench_replace_load.php
 */

const LEAF_COUNT = 1024;
const MID_WIDTH = 8192;
const MID_REFS = 8;
const HEAVY_WIDTH = 8192;
const HEAVY_REFS = 40;
const REPLACE_KEYS = 512;
const PLACEHOLDERS_PER_FORMULA = 6;
const WARMUP = 2;
const ITERATIONS = 5;

function buildSheetAndReplacements(): array
{
    $replacements = [];
    for ($i = 0; $i < REPLACE_KEYS; $i++) {
        $key = "__R{$i}__";
        if ($i % 17 === 0) {
            $replacements[$key] = "T{$i}";
        } else {
            $replacements[$key] = ($i % 97) + 1;
        }
    }

    $cells = [];
    for ($i = 0; $i < LEAF_COUNT; $i++) {
        $cells["L{$i}"] = (string) ($i % 97);
    }

    for ($i = 0; $i < MID_WIDTH; $i++) {
        $args = [];
        for ($j = 0; $j < MID_REFS; $j++) {
            $args[] = 'L' . (($i + $j) % LEAF_COUNT);
        }
        $ph = [];
        for ($j = 0; $j < PLACEHOLDERS_PER_FORMULA; $j++) {
            $k = ($i + $j * 3) % REPLACE_KEYS;
            if ($k % 17 === 0) {
                $k = ($k + 1) % REPLACE_KEYS;
            }
            $ph[] = "__R{$k}__";
        }
        $cells["M{$i}"] = '=SUM(' . implode(',', $args) . ',' . implode(',', $ph) . ')';
    }

    for ($i = 0; $i < 32; $i++) {
        $k = ($i * 17) % REPLACE_KEYS;
        $cells["C{$i}"] = "=\"id=\"&__R{$k}__&M{$i}";
    }

    for ($i = 0; $i < HEAVY_WIDTH; $i++) {
        $args = [];
        for ($j = 0; $j < HEAVY_REFS; $j++) {
            $args[] = 'M' . (($i + $j) % MID_WIDTH);
        }
        for ($j = 0; $j < PLACEHOLDERS_PER_FORMULA; $j++) {
            $k = ($i * 5 + $j) % REPLACE_KEYS;
            if ($k % 17 === 0) {
                $k = ($k + 1) % REPLACE_KEYS;
            }
            $args[] = "__R{$k}__";
        }
        $cells["H{$i}"] = '=SUM(' . implode(',', $args) . ')';
    }

    return [$cells, $replacements];
}

function measure(int $n, callable $fn): array
{
    $totalNs = 0;
    $minNs = PHP_INT_MAX;
    $last = null;
    for ($i = 0; $i < $n; $i++) {
        $t0 = hrtime(true);
        $last = $fn();
        $elapsed = hrtime(true) - $t0;
        $totalNs += $elapsed;
        if ($elapsed < $minNs) {
            $minNs = $elapsed;
        }
    }
    return [
        'avg_ms' => ($totalNs / $n) / 1e6,
        'min_ms' => $minNs / 1e6,
        'total_ms' => $totalNs / 1e6,
        'last' => $last,
    ];
}

echo "=== bench_replace_load (PHP) ===" . PHP_EOL;
echo 'PHP ' . PHP_VERSION . PHP_EOL;

$buildStart = hrtime(true);
[$cells, $replacements] = buildSheetAndReplacements();
$buildMs = (hrtime(true) - $buildStart) / 1e6;

$placeholderHits = 0;
foreach ($cells as $expr) {
    $placeholderHits += substr_count($expr, '__R');
}

echo 'cells in:           ' . count($cells) . PHP_EOL;
echo 'replacement keys:   ' . count($replacements) . PHP_EOL;
echo "placeholder tokens: ~{$placeholderHits}" . PHP_EOL;
echo sprintf("sheet build:        %.3f ms%s", $buildMs, PHP_EOL);
echo 'warmup=' . WARMUP . ', iterations=' . ITERATIONS . PHP_EOL;
echo PHP_EOL;

for ($i = 0; $i < WARMUP; $i++) {
    calc_spreadsheet($cells, $replacements);
    calc_spreadsheet($cells, []);
}

$with = measure(ITERATIONS, static fn () => calc_spreadsheet($cells, $replacements));
$without = measure(ITERATIONS, static fn () => calc_spreadsheet($cells, []));

$sample = $with['last']['H0'] ?? null;
if (!is_float($sample) && !is_int($sample)) {
    fwrite(STDERR, "expected numeric H0, got: " . var_export($sample, true) . PHP_EOL);
    exit(1);
}

echo "--- with replacements ---" . PHP_EOL;
echo sprintf(
    "  avg %.3f ms | min %.3f ms | total %.3f ms (n=%d)%s",
    $with['avg_ms'],
    $with['min_ms'],
    $with['total_ms'],
    ITERATIONS,
    PHP_EOL
);
echo '  cells out: ' . count($with['last']) . PHP_EOL;
echo '  sample H0 = ' . var_export($sample, true) . PHP_EOL;
echo PHP_EOL;

echo "--- without replacements ---" . PHP_EOL;
echo sprintf(
    "  avg %.3f ms | min %.3f ms | total %.3f ms (n=%d)%s",
    $without['avg_ms'],
    $without['min_ms'],
    $without['total_ms'],
    ITERATIONS,
    PHP_EOL
);
echo PHP_EOL;

$ratio = $with['avg_ms'] / max($without['avg_ms'], 1e-9);
echo sprintf(
    "replace path / empty path (avg): %.2fx (%.1f ms vs %.1f ms)%s",
    $ratio,
    $with['avg_ms'],
    $without['avg_ms'],
    PHP_EOL
);
