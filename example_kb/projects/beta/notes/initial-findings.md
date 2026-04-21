---
title: "Adaptive Kalman filter — initial benchmarks"
status: completed
author: Marco Bianchi
tags:
  - benchmarks
  - kalman-filter
  - latency
priority: medium
batch: "3b-rerun"
algorithm: Adaptive Kalman Filter
dataset: alpha-calibration-2031-q3
convergence_ms: 847
funding:
  - "internal"
---
# Adaptive Kalman Filter — Initial Benchmarks

## Setup

Tested the adaptive Kalman filter implementation against the Alpha calibration dataset from Q3 2031. The filter was configured with a 4-state model (wavelength, intensity, drift rate, noise floor) and run on a simulated 64-pixel array.

## Results

| Metric | Value |
|--------|-------|
| Mean convergence time | 847 ms |
| 95th percentile | 1,230 ms |
| Correction accuracy | 94.2% |
| Memory usage | 128 MB |

The filter converges reliably but 847ms mean latency is far too slow for the 50ms target. The bottleneck is the matrix inversion step, which scales as O(n³) with the number of pixels. For 64 pixels this is already impractical in real-time.

## Conclusion

The Kalman filter works as a batch correction tool but cannot serve as the real-time engine. We need a dimensionality reduction step upstream — either PCA or wavelet decomposition — to bring the effective state space down to something manageable. Chiara will investigate the wavelet approach.
