---
title: "Q2 2032 — Integration Sprint"
author: Giulia Ferretti
tags:
  - progress
  - integration
draft: false
date: "2032-06-30"
status: published
---
# Q2 2032 — Integration Sprint

Big quarter. We finally connected Alpha and Beta into a single pipeline, and it works. Not perfectly, not in real-time yet, but it works.

## The Pipeline

1. Sensor array captures raw data (Alpha hardware)
2. Chiara's wavelet denoiser cleans the signal (~15ms)
3. Marco's Kalman filter computes the correction (~180ms)
4. MetaMat Controller applies the correction to the pixel array (~10ms)
5. Total loop time: ~205ms

The 50ms target is still out of reach, but 205ms is already useful for many diagnostic applications where you're measuring over minutes, not milliseconds. For a first integration, I'm thrilled.

## What's Next

Chiara is optimizing the wavelet step — she thinks she can get it under 5ms with a GPU implementation. Marco is exploring whether we can skip the full Kalman update on frames where the drift is below a threshold. REMO is running the 1000-cycle endurance test overnight.

The Photonics Europe talk is in two months. We have a story to tell.
