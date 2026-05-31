---
name: ml-model-debugging
description: >-
  Generic ML model debugging workflow and ML contract checklist. Use for tasks
  involving ML models, transformers, tokenizers, checkpoints, embeddings,
  retrieval, reranking, similarity search, model inference/serving, incorrect
  predictions, degraded metrics, data/model mismatch, numerical instability,
  performance regressions, or unexpected behavior in machine learning systems.
---

# ML Model Debugging

Debug ML issues systematically. Start by identifying where the failure enters the pipeline, then isolate the smallest reproducible case.

## Workflow

1. Restate the observed problem and the expected behavior or metric.
2. Identify the affected stage: data, preprocessing, model, training, evaluation, inference, or serving.
3. Reproduce with a small fixed input, seed, checkpoint, and configuration; for training/distributed code, first reduce to the smallest single-process or single-rank case before adding communication or batching complexity.
4. Compare against a known-good baseline, previous checkpoint, reference implementation, or manual expectation.
5. Inspect intermediate artifacts until the first divergence is found.
6. Make one change at a time and re-run the smallest relevant test.

## ML Contract Check

Before implementing or verifying ML inference, ranking, similarity, embedding, retrieval, reranking, or evaluation tasks:

1. Identify the intended abstraction named by the task: package, wrapper, model card, local metadata, checkpoint, tokenizer/processor, config, or reference implementation.
2. Prefer the named abstraction's contract over raw lower-level model calls when the task names a higher-level package or wrapper.
3. Inspect whether query/document inputs require different preprocessing, prompts/instructions, tokenization, pooling, normalization, truncation, dtype/device handling, or postprocessing.
4. Preserve exact model identity: model name, revision/hash, checkpoint path, tokenizer/processor version, adapter/LoRA, and config.
5. If two plausible conventions produce different outputs or rankings, do not verify against only one. Resolve which convention is authoritative from discoverable metadata, docs, examples, or the named package behavior.
6. In final verification, exercise the same external contract expected by callers, not only the helper code used during implementation.
7. For training, autograd, distributed, or numerical-parity tasks, compare losses, logits/outputs, parameter gradients, activation gradients, and scaling/reduction behavior against a simple reference path before trusting static checks.

## Debugging Checklist

Consider the relevant areas for the task:

- Data quality: missing labels, corrupt samples, duplicates, leakage, skew, class imbalance, or train/test contamination
- Preprocessing/contract: tokenization, normalization, prompts/instructions, pooling, resizing, feature order, dtype, padding, truncation, masks, postprocessing, and train/inference parity

- Configuration: wrong checkpoint, model version, tokenizer/processor mismatch, feature schema mismatch, or stale artifacts
- Model behavior: incorrect output shape, activation range, loss target format, frozen/unfrozen layers, dropout/batch norm mode, or thresholding
- Numerical stability: NaN/Inf values, exploding/vanishing gradients, precision issues, overflow/underflow, unstable softmax/loss, or bad initialization
- Training loop: seed control, optimizer state, learning rate, scheduler, gradient accumulation, clipping, mixed precision, and distributed synchronization
- Evaluation: metric implementation, averaging method, label mapping, calibration, ranking cutoff, and batch-size-dependent behavior
- Inference/serving: serialization, device placement, batching, concurrency, timeout, memory limits, request schema, and postprocessing
- Reproducibility: fixed random seeds, deterministic settings, pinned versions, logged configs, and artifact hashes
- Performance: latency, throughput, memory, data loading bottlenecks, CPU/GPU utilization, and unnecessary copies

## Useful Tactics

- Run a single example end-to-end and print shapes, dtypes, devices, ranges, and key intermediate values.
- For ranking/retrieval, compare candidate rankings under plausible canonical wrappers before selecting one.
- For training-step implementations, verify a tiny reference calculation: loss value, label alignment, reduction/scaling, and representative gradients.
- Overfit a tiny dataset; failure usually indicates a model, loss, optimizer, or preprocessing bug.
- Test invariants: identical input gives identical output in eval mode, probabilities sum as expected, masks hide padded/invalid tokens, and labels align with predictions.
- Compare train vs inference preprocessing and ensure the same vocabulary, feature schema, normalization, and postprocessing are used.
- Bisect changes across commits, checkpoints, data versions, or config changes when a regression is suspected.
- Prefer concrete evidence over guesses: save minimal inputs, outputs, configs, logs, and commands that reproduce the issue.

## Output Format

Return a concise diagnostic plan or findings summary:

```markdown
## Problem
- Observed:
- Expected:

## Likely failure point
- Stage:
- Rationale:

## Checks performed / recommended
- [ ] Check: command, input, or artifact to inspect and what result would confirm/refute it

## Findings
- ...

## Next action
- ...
```

Tailor the checklist to the model and task. Do not assume a specific architecture, tokenizer, framework, or deployment environment unless the task provides one.
