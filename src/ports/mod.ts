// Ports layer — trait definitions (input/output contracts).

export interface ProviderPort {
  fetchModelList(): Promise<string[]>;
  evaluateModel(modelId: string): Promise<number>;
}

export interface StoragePort {
  saveResult(result: unknown): Promise<void>;
  loadResults(): Promise<unknown[]>;
}

export interface NotifierPort {
  notify(message: string): Promise<void>;
}
