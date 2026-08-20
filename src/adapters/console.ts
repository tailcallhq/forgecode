import { NotifierPort } from '../ports';

export class ConsoleNotifier implements NotifierPort {
  async notify(message: string): Promise<void> {
    console.log(message);
  }
}
