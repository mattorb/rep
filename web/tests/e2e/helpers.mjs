import { once } from "node:events";
import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../../..");
const binary = path.join(repo, "target", "debug", "rep");

export function fixture(name) {
  return path.join(repo, "tests", "fixtures", "web", name);
}

export async function launchRep(name) {
  const child = spawn(binary, ["--web", "--no-open", fixture(name)], {
    cwd: repo,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let diagnostics = "";
  let output = "";
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    output += chunk;
  });
  const url = await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(new Error(`Timed out waiting for Rep URL. stderr:\n${diagnostics}`));
    }, 8_000);
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      diagnostics += chunk;
      const match = diagnostics.match(/^Review URL: (http:\/\/[^\s]+)$/m);
      if (match) {
        clearTimeout(timeout);
        resolve(match[1]);
      }
    });
    child.once("exit", (code) => {
      clearTimeout(timeout);
      reject(new Error(`Rep exited before launch (code ${code}). stderr:\n${diagnostics}`));
    });
  });
  return {
    child,
    diagnostics: () => diagnostics,
    output: () => output,
    url,
  };
}

export async function waitForRep(running) {
  if (running.child.exitCode === null) await once(running.child, "exit");
  if (running.child.exitCode !== 0) {
    throw new Error(
      `Rep exited with ${running.child.exitCode}. stderr:\n${running.diagnostics()}`,
    );
  }
  return running.output();
}

export async function finishRep(page, running, action = "discard") {
  if (running.child.exitCode === null) {
    const exited = once(running.child, "exit");
    await page.evaluate(async (action) => {
      const response = await fetch(new URL(`api/${action}`, window.location.href), {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: "{}",
      });
      if (!response.ok) throw new Error(`finish failed: ${response.status}`);
    }, action);
    if (running.child.exitCode === null) {
      await exited;
    }
  }
  await waitForRep(running);
}

export async function openPlan(page, name) {
  const running = await launchRep(name);
  try {
    await page.goto(running.url);
    const frame = page.frameLocator("#plan");
    await frame.locator("body").waitFor({ state: "attached" });
    await page.waitForFunction(() =>
      ["ready", "empty"].includes(window.__repTest?.state?.status),
    );
    return { frame, running };
  } catch (error) {
    if (running.child.exitCode === null) {
      const exited = once(running.child, "exit");
      running.child.kill("SIGTERM");
      await exited;
    }
    throw new Error(
      `${error.message}\nRep exited with ${running.child.exitCode}. stderr:\n${running.diagnostics()}`,
      { cause: error },
    );
  }
}
