// Removed WASM module declaration since we're now using pure TypeScript

interface DurableObjectId {}

interface DurableObjectStub {
  fetch(input: Request | string, init?: RequestInit): Promise<Response>;
}

interface DurableObjectNamespace {
  idFromName(name: string): DurableObjectId;
  get(id: DurableObjectId): DurableObjectStub;
}

interface DurableObjectState {}
