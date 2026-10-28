// LLMBot with ANY provider: you pass `complete(prompt) => text`. No API keys live in the SDK.
import { run } from "@sharingan/sdk";
import { LLMBot } from "@sharingan/sdk/bots";

const complete = async (prompt: string): Promise<string> => {
  // e.g. fetch your provider here, or use its SDK; return the model's text
  throw new Error("plug in your provider");
};
await run(new LLMBot(complete), process.env.ARENA_URL!, process.env.ARENA_TOKEN!);
