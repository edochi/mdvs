---
title: "Gamma — What We'd Do Differently"
status: archived
author: Marco Bianchi
tags:
  - post-mortem
  - lessons
author's_note: I pushed for killing this project earlier than we did. In hindsight, the warning signs were there from the first outdoor test.
priority: low
---
# Gamma — What We'd Do Differently

This is a companion to the formal post-mortem. More personal, less diplomatic.

## On Scope

We tried to jump from "works on a bench" to "works in a city" in one step. There should have been an intermediate stage — a controlled outdoor environment, maybe a covered balcony or a greenhouse. Somewhere with real air but manageable humidity.

## On Timelines

Giulia wanted to push through. I understand why — we had a partnership lined up with the city's environmental agency and the pressure to deliver was real. But two months of debugging a materials problem with software tools was not productive.

## On Reuse

The signal processing work from Gamma wasn't wasted. The adaptive filtering techniques I developed for compensating humidity artifacts became the starting point for Beta's reconfiguration engine. The path from "failed smog sensor" to "real-time metamaterial tuning" was shorter than expected.

## On REMO

REMO's environmental monitoring data from the outdoor tests was the most useful output of the entire project. His obsessive humidity logging is what eventually led to the A-031 experiment on Alpha. Sometimes the side effect is the main effect.
