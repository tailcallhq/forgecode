import { ProviderPort } from '../ports';

export class GithubApiAdapter implements ProviderPort {
  async fetchModelList(): Promise<string[]> {
    return [];
  }

  async evaluateModel(_modelId: string): Promise<number> {
    return 0;
  }
}
