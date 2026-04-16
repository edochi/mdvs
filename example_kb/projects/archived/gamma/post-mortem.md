---
title: "Project Gamma — Post-Mortem"
status: archived
author: Giulia Ferretti
tags:
  - post-mortem
  - atmospheric
  - discontinued
date: "2031-03-15"
priority: medium
---
# Project Gamma — Post-Mortem

## What Was Gamma?

Gamma was an attempt to use our metamaterial platform for atmospheric particulate detection — essentially a portable smog sensor for urban environments. The idea was to tune a sensor array to detect specific particulate signatures (PM2.5, PM10, NOx-bound particles) using the same programmable pixel approach we use for biosensing.

## What Went Wrong

The lab prototype worked beautifully. We could distinguish particle types in controlled air samples with 89% accuracy. But field trials in Turin were a disaster. The city's humidity (often >80% in winter) caused condensation on the metamaterial surface, which completely disrupted the resonance patterns. The sensor would report wildly inaccurate readings within 20 minutes of outdoor exposure.

We spent two months trying to solve this with hydrophobic coatings, heated enclosures, and software compensation. Nothing worked well enough for a practical device.

## Decision

Killed the project in March 2031. The humidity problem is fundamentally a materials science challenge, not a signal processing one, and we don't have the expertise or budget to tackle it. The core insight — that metamaterial sensors are exquisitely sensitive to environmental conditions — actually informed REMO's environmental sensitivity work on Alpha.

## Lessons Learned

1. Lab conditions are not field conditions. Test outdoors early.
2. Humidity is the enemy of surface-sensitive optical devices.
3. Know when to stop. Two months of failed fixes was probably one month too many.
