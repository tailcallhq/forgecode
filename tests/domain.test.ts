import { describe, it } from 'node:test';
import assert from 'node:assert';
import { ScoringEngine, BountyRule } from '../src/domain';

describe('ScoringEngine', () => {
  it('should evaluate all rules', () => {
    const engine = new ScoringEngine();
    engine.addRule({ id: 'min', description: 'min score', weight: 1, condition: (s) => s >= 50 });
    assert.strictEqual(engine.evaluate(60), true);
    assert.strictEqual(engine.evaluate(40), false);
  });

  it('should compute weighted average', () => {
    const engine = new ScoringEngine();
    assert.strictEqual(engine.computeWeightedScore([80, 90, 100]), 90);
  });
});
