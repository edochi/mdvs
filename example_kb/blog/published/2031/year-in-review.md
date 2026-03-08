---
title: "2031 in Review — A Year of Calibration"
author: Giulia Ferretti
tags:
  - year-in-review
  - retrospective
draft: false
date: "2031-12-20"
status: published
---
# 2031 in Review — A Year of Calibration

What a year. We started 2031 with three projects and ended with two — but the two that survived are stronger for it.

## Highlights

**Alpha** hit its stride over the summer. REMO ran over 50 calibration experiments with mechanical precision (literally). The big discovery was the humidity-drift correlation — something we should have caught earlier but didn't, because nobody was logging environmental data at four decimal places. Thank you, REMO.

**Beta** got a new team member! Chiara joined in September and immediately started making the Kalman filter less terrible. Her wavelet preprocessing idea is promising — early results suggest we can bring latency down from 847ms to under 200ms.

**Gamma** was killed in March. It hurt, but it was the right call. The full post-mortem is in `projects/archived/gamma/`.

## Lowlights

- The NanoFab B2 substrate batch was defective. Cost us 6 weeks of waiting.
- I still haven't fixed the frontmatter on the older blog posts. Sara is patient but not infinitely so.
- REMO filed a maintenance request for the coffee machine as an "equipment protocol violation report." We are still sorting out the ticketing system.

## Looking Ahead

2032 is about integration. Can we connect Alpha's calibration data to Beta's correction engine and get a closed-loop system working? If yes, we have a real product. If not, we have two very well-documented subsystems. Either way, the documentation is excellent.
