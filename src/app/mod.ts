// App layer — composition root. Wires adapters to domain.

import { ScoringEngine } from '../domain';
import { ProviderPort, StoragePort, NotifierPort } from '../ports';

export class App {
  engine: ScoringEngine;
  provider: ProviderPort;
  storage: StoragePort;
  notifier: NotifierPort;

  constructor(
    provider: ProviderPort,
    storage: StoragePort,
    notifier: NotifierPort,
  ) {
    this.engine = new ScoringEngine();
    this.provider = provider;
    this.storage = storage;
    this.notifier = notifier;
  }

  async runEvaluation(): Promise<void> {
    const models = await this.provider.fetchModelList();
    for (const modelId of models) {
      const score = await this.provider.evaluateModel(modelId);
      const passed = this.engine.evaluate(score);
      await this.notifier.notify(`Model ${modelId}: ${passed ? 'PASS' : 'FAIL'} (${score})`);
    }
  }
}
