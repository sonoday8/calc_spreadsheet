<?php

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
// Optional: calc_spreadsheet($cells, $min_layer_width, $min_layer_work)
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
