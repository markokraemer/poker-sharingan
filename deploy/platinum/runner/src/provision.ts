// One-off: build the Platinum templates from the pushed images. Run by hand, never from the request path.
//   PT_TOKEN=... PT_API_URL=... node --experimental-strip-types src/provision.ts sharingan-bot [sharingan-arena]
// Needs a key with templates:write. Images must already be pushed to a registry Platinum can pull anonymously (DESIGN.md G7).
// (Local mode has no templates: the "template" is the image built by deploy/local/e2e.sh.)
import { readFileSync } from "node:fs";
import { Platinum, Template } from "@platinum-dev/sdk";

const pt = new Platinum({ url: process.env.PT_API_URL ?? "https://api.platinum.dev", token: process.env.PT_TOKEN });
const names = process.argv.slice(2);
if (!names.length) { console.error("usage: provision.ts <template json basename>..."); process.exit(2); }

for (const n of names) {
  const spec = JSON.parse(readFileSync(new URL(`../../templates/${n}.json`, import.meta.url), "utf8"));
  if (String(spec.image).includes("REPLACE_ORG")) throw new Error(`${n}: set the registry org in templates/${n}.json`);
  const { _comment, image, name, ...opts } = spec;
  console.log(`building ${name} from ${image} ...`);
  const tpl = await Template.fromImage(image).build(pt, { name, ...opts });
  console.log("ready:", tpl);
}
