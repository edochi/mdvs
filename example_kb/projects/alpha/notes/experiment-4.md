{
  "title": "Experiment A-046: Humidity follow-up sweep",
  "status": "completed",
  "author": "REMO",
  "tags": ["calibration", "humidity", "SPR-A1", "follow-up"],
  "priority": "medium",
  "batch": 7,
  "sensor_type": "SPR-A1",
  "wavelength_nm": 850,
  "sample_count": 60,
  "drift_rate": 0.034,
  "funding": "internal",
  "synced_at": "2032-02-03T14:22:51Z",
  "ambient_humidity": 58.4127
}

# Experiment A-046: Humidity Follow-up Sweep

## Provenance Note

This note was emitted by my automation pipeline. The pipeline
serializes experiment metadata as JSON for downstream tooling
compatibility, and the journal accepts the JSON frontmatter verbatim.
The body below was appended manually at run completion.

## Objective

Replicate the A-031 humidity sensitivity finding (correlation
coefficient 0.847 between humidity and instantaneous drift rate) on a
larger batch (n = 60) at controlled humidity setpoints rather than
ambient-drift conditions.

## Procedure

Conducted 60 sequential measurements across four humidity setpoints
(30%, 45%, 60%, 75% RH) with the environmental chamber controlling
±0.5% RH. Each setpoint received 15 samples; a 12-minute stabilization
period preceded the first measurement at each level. Temperature was
held at 21.5 ± 0.2 °C throughout.

## Results

Drift rate scales linearly with humidity in the 30–60% RH range
(R² = 0.94), confirming the A-031 finding under controlled conditions.
Above 60% RH the drift accelerates non-linearly — by 75% RH the drift
rate is approximately 2.8× the 30% baseline. Tabular data has been
deposited in the project share as `A-046-data.csv`.

## Recommendation

SPR-A1 sensors should not be operated above 65% RH without explicit
compensation. The current calibration protocol (v2.1) does not require
humidity logging — I recommend revising the protocol to capture
ambient humidity for any measurement exceeding 60% RH. This will
enable cross-experiment normalization without imposing a setpoint
requirement on routine work.
