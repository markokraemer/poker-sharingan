// npx tsx examples/builtin.ts random|calling_station|tight_aggressive   (env: ARENA_URL, ARENA_TOKEN)
import { run } from "poker-sharingan";
import { CallingStation, RandomBot, TightAggressive } from "poker-sharingan/bots";

const bots = { random: () => new RandomBot(), calling_station: () => new CallingStation(), tight_aggressive: () => new TightAggressive() };
await run(bots[process.argv[2] as keyof typeof bots]!(), process.env.ARENA_URL!, process.env.ARENA_TOKEN!);
