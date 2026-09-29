import { readdirSync, statSync, writeFileSync, mkdirSync } from "fs";
import { join, relative } from "path";

const root = import.meta.dir;
const pub = join(root, "public");

function walk(dir) {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? walk(p) : [p];
  });
}

export function writeManifest() {
  const list = walk(pub).map((p) => relative(root, p).split("\\").join("/"));
  const lines = list.map((p, i) => `import f${i} from "./${p}" with { type: "file" };`);
  lines.push("export const files = {");
  list.forEach((p, i) => lines.push(`  ${JSON.stringify(p.slice("public".length))}: f${i},`));
  lines.push("};", "");
  writeFileSync(join(root, "embedded.js"), lines.join("\n"));
  return list.length;
}

if (import.meta.main) {
  console.log(`embedded ${writeManifest()} files`);
  if (!process.argv.includes("--manifest-only")) {
    mkdirSync(join(root, "dist"), { recursive: true });
    const targets = [
      ["bun-linux-x64-baseline", "dist/FallBeans-linux-x64"],
    ];
    for (const [target, out] of targets) {
      const proc = Bun.spawnSync(["bun", "build", "server.js", "--compile", "--minify", `--target=${target}`, `--outfile=${out}`], { cwd: root, stdout: "inherit", stderr: "inherit" });
      if (proc.exitCode !== 0) { console.error(`build failed: ${target}`); process.exit(1); }
    }
  }
}
