import { StoragePort } from '../ports';

export class CsvAdapter implements StoragePort {
  async saveResult(_result: unknown): Promise<void> {
    return;
  }

  async loadResults(): Promise<unknown[]> {
    return [];
  }
}
