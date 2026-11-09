import { Agent, Dispatcher } from "undici";
import { perSlot } from "../slot-scope";

// The connections of fetch in the light host: one pool per slot, as each account had in a process of its own. With the
// one pool of the process, undici puts every account on one HTTP/2 connection per Microsoft host, and the chat list of
// one account waits behind the others until its 20 s timeout.
class PerSlot extends Dispatcher {
  constructor(private readonly agents: () => Agent) {
    super();
  }

  dispatch(options: Dispatcher.DispatchOptions, handler: Dispatcher.DispatchHandler) {
    return this.agents().dispatch(options, handler);
  }
}

export const slotConnections = (make: () => Agent = () => new Agent()): Dispatcher => new PerSlot(perSlot(make));
