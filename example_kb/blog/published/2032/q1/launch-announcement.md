---
title: "New Equipment: MetaMat Controller v2.4"
author: Sara Dell'Acqua
tags:
  - equipment
  - announcement
draft: false
date: "2032-01-18"
status: published
---
# New Equipment: MetaMat Controller v2.4

The new MetaMat Controller (v2.4) is installed and operational. This replaces the v2.1 unit that has been increasingly unreliable since October.

## What Changed

- Faster pixel addressing: 64 pixels can now be individually tuned in under 10ms (v2.1 took ~25ms)
- Improved thermal management — the old unit overheated during long calibration runs
- New API (v2.x compatible, but some function signatures changed — see the updated SDK docs)

## Access

Same location (bench 3), same login procedure. If you were using the old API directly, check the migration guide in the SDK repository. REMO has already updated his automation scripts.

## Important

Do NOT attempt to run the v3.x firmware on this unit. The hardware is not compatible and you will brick it. Yes, the v3.x changelog looks tempting. No, you cannot install it. I have disabled the firmware update menu. Do not try to re-enable it.
