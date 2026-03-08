---
title: Project Beta — Real-Time Reconfiguration Engine
status: active
author: Marco Bianchi
tags:
  - signal-processing
  - adaptive-control
  - software
priority: high
---
# Project Beta

Beta is the software counterpart to Alpha. The goal is to build a real-time engine that can retune metamaterial pixels on the fly, compensating for calibration drift and environmental noise without manual intervention.

## Architecture

The engine takes raw sensor readings as input, compares them to the expected resonance profile, and computes the correction signal needed to retune each pixel. The core challenge is latency: the correction must be applied within one measurement cycle (~50ms) to be useful.

## Current Status

The adaptive Kalman filter approach works but is too slow for real-time use on large arrays (>32 pixels). We are exploring wavelet-based denoising as a preprocessing step to reduce the dimensionality of the input signal before feeding it to the filter. Chiara is working on this.

## Team

- Marco Bianchi (lead)
- Chiara Russo (intern — wavelet denoising)
