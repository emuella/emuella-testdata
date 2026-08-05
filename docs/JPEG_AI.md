# JPEG AI data and model boundary

JPEG AI introduces training and learned-weight concerns that ordinary codec
test inputs do not. Keep these artifact classes separate:

```text
jpeg-ai/conformance/
jpeg-ai/validation/
jpeg-ai/training/
jpeg-ai/models/
```

A pack permitted for conformance or benchmarking is not automatically eligible
for training. A training pack is not automatically eligible for published
weights. Every manifest records `ml_training` and `weights_redistribution`
explicitly, with unknown treated as unavailable.

The future Apache-2.0 implementation should load model assets through a stable
interface rather than requiring differently licensed weights to be compiled
into Rust source. If weights develop a different release cadence, governance,
or licence, a separate `emuella-models` distribution can be introduced without
splitting the test-data catalogue.

Do not use ISO conformance attachments, purpose-restricted imagery, personal
data, or click-through datasets for training merely because the bytes are
available to a developer.
