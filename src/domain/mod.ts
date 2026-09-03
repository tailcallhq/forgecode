// Domain layer — pure evaluation logic, no framework dependencies.

export interface EvaluationModel {
  modelId: string;
  score: number;
  metadata: Record<string, unknown>;
}

export interface BountyRule {
  id: string;
  description: string;
  weight: number;
  condition: (score: number) => boolean;
}

export class ScoringEngine {
  rules: BountyRule[] = [];

  addRule(rule: BountyRule): void {
    this.rules.push(rule);
  }

  evaluate(score: number): boolean {
    return this.rules.every((rule) => rule.condition(score));
  }

  computeWeightedScore(scores: number[]): number {
    if (scores.length === 0) return 0;
    return scores.reduce((a, b) => a + b, 0) / scores.length;
  }
}
