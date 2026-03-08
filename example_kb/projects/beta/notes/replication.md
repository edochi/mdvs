---
title: Wavelet denoising — replication attempt
author: Chiara Russo
tags:
  - 1
  - 2
  - 3
algorithm: Wavelet Decomposition
dataset: alpha-calibration-2031-q3
---
# Wavelet Denoising — Replication Attempt

Tried to replicate the results from Nakamura et al. (2030) on spectral denoising using Daubechies wavelets. Used the same Alpha calibration dataset that Marco benchmarked the Kalman filter on.

## What I Did

Applied a 4-level wavelet decomposition (db4) to the raw sensor signal, discarded the detail coefficients below a threshold, and reconstructed. Fed the denoised signal into the Kalman filter.

## Results

It kind of works? The denoised signal is definitely cleaner and the Kalman filter converges faster (around 200ms instead of 847ms). But the threshold selection is tricky — too aggressive and you lose real signal features, too conservative and you don't gain much.

I need to talk to Marco about automatic threshold selection. The paper uses a method based on the noise floor estimate but I'm not sure how to get that from our data.

## TODO

- Figure out threshold selection
- Test on the B2 dataset (once it exists)
- Actually add proper frontmatter to this note (sorry Sara)
