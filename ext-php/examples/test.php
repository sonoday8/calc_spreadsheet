<?php

echo "=== placeholder replace ===" . PHP_EOL;
$replaced = calc_spreadsheet(
    [
        'A1' => '__NAME__',
        'B1' => 'NAME',
        'C1' => '__UNKNOWN__',
        'D1' => '=__RATE__*2',
        'E1' => '=F1+1',
        'F1' => '=__A1__',
    ],
    [
        '__NAME__' => 'Alice',
        '__RATE__' => 10,
        '__A1__' => 7,
    ]
);
foreach (['A1', 'B1', 'C1', 'D1', 'E1', 'F1'] as $cell) {
    echo $cell . ' = ';
    var_export($replaced[$cell] ?? null);
    echo PHP_EOL;
}

$unknown = calc_spreadsheet(['Z1' => '=__UNKNOWN__*2', 'Z2' => '=__UNKNOWN__+1']);
echo 'unknown in formula Z1 = ';
var_export($unknown['Z1'] ?? null); // 0 (missing name like =Z99)
echo PHP_EOL;
echo 'unknown in formula Z2 = ';
var_export($unknown['Z2'] ?? null); // 1
echo PHP_EOL;

echo "=== invalid replacement keys warn ===" . PHP_EOL;
$warnings = [];
set_error_handler(static function (int $errno, string $errstr) use (&$warnings): bool {
    if ($errno === E_USER_WARNING) {
        $warnings[] = $errstr;
        return true;
    }
    return false;
});
$warned = calc_spreadsheet(
    ['A1' => '=__OK__+1'],
    [
        '__OK__' => 3,
        'bad' => 9,
        '__name__' => 'x',
    ]
);
restore_error_handler();
echo 'A1 = ';
var_export($warned['A1'] ?? null); // 4
echo PHP_EOL;
echo 'warnings: ' . count($warnings) . PHP_EOL;
foreach ($warnings as $w) {
    echo $w . PHP_EOL;
}
if (count($warnings) !== 1) {
    fwrite(STDERR, "expected 1 E_USER_WARNING\n");
    exit(1);
}
if (
    strpos($warnings[0], 'bad') === false
    || strpos($warnings[0], '__name__') === false
    || strpos($warnings[0], 'ignored invalid replacement key') === false
) {
    fwrite(STDERR, "warning text missing expected keys\n");
    exit(1);
}
if (($warned['A1'] ?? null) != 4) {
    fwrite(STDERR, "expected A1 == 4 after ignoring invalid keys\n");
    exit(1);
}
echo PHP_EOL;

echo "=== mixed sheet bench ===" . PHP_EOL;

const LEAF_COUNT = 256;
const LIGHT_WIDTH = 256;
const LIGHT_REFS = 3;
const LIGHT_DEPTH = 16;
const HEAVY_WIDTH = 4096;
const HEAVY_REFS = 80;

$buildStart = hrtime(true);
$cells = [];

for ($i = 0; $i < LEAF_COUNT; $i++) {
    $cells["L0_{$i}"] = (string) ($i % 97);
}

for ($depth = 1; $depth <= LIGHT_DEPTH; $depth++) {
    $prev = $depth - 1;
    $prevCount = $prev === 0 ? LEAF_COUNT : LIGHT_WIDTH;
    for ($i = 0; $i < LIGHT_WIDTH; $i++) {
        $args = [];
        for ($j = 0; $j < LIGHT_REFS; $j++) {
            $args[] = 'L' . $prev . '_' . (($i + $j) % $prevCount);
        }
        $cells["L{$depth}_{$i}"] = '=SUM(' . implode(', ', $args) . ')';
    }
}

$heavyDepth = LIGHT_DEPTH + 1;
for ($i = 0; $i < HEAVY_WIDTH; $i++) {
    $args = [];
    for ($j = 0; $j < HEAVY_REFS; $j++) {
        $args[] = 'L' . LIGHT_DEPTH . '_' . (($i + $j) % LIGHT_WIDTH);
    }
    $cells["L{$heavyDepth}_{$i}"] = '=SUM(' . implode(', ', $args) . ')';
}

$buildMs = (hrtime(true) - $buildStart) / 1e6;

$calcStart = hrtime(true);
// calc_spreadsheet($cells, $replacements = [], $min_layer_width, $min_layer_work)
$result = calc_spreadsheet($cells);
$calcMs = (hrtime(true) - $calcStart) / 1e6;

$sampleKey = "L{$heavyDepth}_0";

echo "cells in:  " . count($cells) . PHP_EOL;
echo "cells out: " . count($result) . PHP_EOL;
echo sprintf("build:     %.3f ms%s", $buildMs, PHP_EOL);
echo sprintf("calc:      %.3f ms%s", $calcMs, PHP_EOL);
echo "sample {$sampleKey} = ";
var_export($result[$sampleKey] ?? null);
echo PHP_EOL;
