// JSON-lines logger. `Logger` is injected everywhere so tests stay quiet (or capture).
export type Logger = (event: string, fields?: Record<string, unknown>) => void;

export const stdoutLog: Logger = (event, fields = {}) => {
  console.log(JSON.stringify({ ts: new Date().toISOString(), event, ...fields }));
};
export const noopLog: Logger = () => {};
