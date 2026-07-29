import { readdir, writeFile } from "node:fs/promises";

const images = (await readdir(new URL("../gallery/", import.meta.url)))
  .filter((name) => name.endsWith(".png"))
  .sort();
const cards = images
  .map(
    (name) =>
      `<figure><img src="${name}" alt="${name}"><figcaption>${name}</figcaption></figure>`,
  )
  .join("\n");
await writeFile(
  new URL("../gallery/index.html", import.meta.url),
  `<!doctype html><meta charset="utf-8"><title>Rep web gallery</title>
<style>body{font:16px system-ui;margin:2rem}figure{margin:0 0 3rem}img{border:1px solid #ccc;max-width:100%}</style>
<h1>Rep HTML review gallery</h1>${cards}\n`,
);
