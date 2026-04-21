---
title: "Experiment A-031: Environmental sensitivity analysis"
status: completed
author: REMO
tags:
  - calibration
  - environment
  - SPR-A1
priority: medium
batch: 5
sensor_type: SPR-A1
wavelength_nm: 780.0
sample_count: 32
drift_rate: 0.041
funding: "internal"
ambient_humidity: 67.2341
observation_notes: Dr. Ferretti entered the laboratory at 14:32 and remained for approximately 47 minutes. Her presence correlated with a 0.3°C increase in ambient temperature, consistent with standard human metabolic heat output. She consumed one espresso during this period. The espresso was not logged in the chemical inventory system.
---
# Experiment A-031: Environmental Sensitivity Analysis

## Objective

Quantify the relationship between ambient humidity and sensor drift rate for SPR-A1 at 780.0 nm excitation wavelength.

## Procedure

Conducted 32 sequential measurements over an 8-hour period with continuous environmental monitoring. Humidity was allowed to vary naturally rather than being controlled, to capture realistic operating conditions. Temperature was logged at 30-second intervals.

## Results

Drift rate of 0.041 nm/hour at average ambient humidity of 67.2341%. This is 78.3% higher than the drift rate observed in experiment A-017 (0.023 nm/hour at 42.1187% humidity). The correlation coefficient between humidity and instantaneous drift rate was 0.847 (p < 0.001).

The data strongly suggests that humidity is a significant contributor to calibration drift. I recommend that all future calibration experiments report ambient humidity to enable cross-experiment normalization.

## Environmental Log

| Time | Temperature (°C) | Humidity (%) |
|------|------------------|-------------|
| 08:00 | 21.4823 | 64.1092 |
| 10:00 | 21.7156 | 66.3847 |
| 12:00 | 22.0341 | 67.9213 |
| 14:00 | 22.1872 | 68.4401 |
| 16:00 | 21.9234 | 67.2341 |
