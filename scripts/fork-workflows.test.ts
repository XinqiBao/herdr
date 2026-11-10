import { describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const load = (name: string): any =>
  Bun.YAML.parse(readFileSync(new URL(`../.github/workflows/${name}.yml`, import.meta.url), "utf8"));
const rolling = load("fork-release");
const build = load("build-artifacts-manual");
const fetchAction = Bun.YAML.parse(readFileSync(new URL("../.github/actions/fetch-zig-deps/action.yml", import.meta.url), "utf8")) as any;

describe("fork release boundaries", () => {
  test("all fork compilation paths require bounded dependency acquisition", () => {
    const ci = load("ci");
    for (const [job, firstCompile, inputs] of [
      [ci.jobs.check, "Run Linux lint", { "include-scalar": "${{ runner.os == 'Linux' && 'true' || 'false' }}" }],
      [ci.jobs["windows-conpty-package"], "Build Herdr", { target: "x86_64-pc-windows-msvc" }],
      [build.jobs["build-linux"], "Build", { target: "${{ matrix.target }}" }],
      [build.jobs["build-macos"], "Build", { target: "${{ matrix.target }}" }],
      [build.jobs["build-windows"], "Build", { target: "x86_64-pc-windows-msvc" }],
    ] as const) {
      const index = job.steps.findIndex((step: any) => step.uses === "./.github/actions/fetch-zig-deps");
      expect(index).toBeGreaterThan(0);
      expect(index).toBeLessThan(job.steps.findIndex((step: any) => step.name === firstCompile));
      const step = job.steps[index];
      expect(step.if).toBe("github.repository == 'XinqiBao/herdr'");
      expect(step["timeout-minutes"]).toBe(5);
      expect(step["continue-on-error"]).toBeUndefined();
      expect(step.with).toEqual(inputs);
    }
    const [restore, fetch, save] = fetchAction.runs.steps;
    expect(restore.with.path).toBe("vendor/libghostty-vt/zig-pkg");
    expect(save.with.path).toBe(restore.with.path);
    expect(save.with.key).toBe("${{ steps.packages.outputs.cache-primary-key }}");
    expect(fetch.if).toBeUndefined(); // verify the selected graph even on cache hits
    expect(save.if).toBe("steps.packages.outputs.cache-hit != 'true'");
  });

  test.skipIf(process.platform === "win32")("fetch retries only the VT graph and fails closed after three attempts", () => {
    const dir = mkdtempSync(join(tmpdir(), "herdr-zig-fetch-"));
    try {
      const log = join(dir, "zig.log");
      writeFileSync(join(dir, "VERSION"), "1.3.2-dev\r\n");
      writeFileSync(join(dir, "rustc"), "#!/bin/sh\nprintf 'host: x86_64-unknown-linux-gnu\\r\\n'\n", { mode: 0o755 });
      writeFileSync(join(dir, "sleep"), "#!/bin/sh\nexit 0\n", { mode: 0o755 });
      writeFileSync(join(dir, "zig"), `#!/bin/sh
printf '%s\\n' "$*" >> "$ZIG_LOG"
test "$(wc -l < "$ZIG_LOG")" -gt "$ZIG_FAILURES"
`, { mode: 0o755 });
      const run = (target: string, scalar: string, failures: string) => {
        writeFileSync(log, "");
        const result = spawnSync("bash", ["-c", fetchAction.runs.steps[1].run], {
          cwd: dir, encoding: "utf8",
          env: { ...process.env, PATH: `${dir}:${process.env.PATH}`, ZIG_LOG: log,
            ZIG_FAILURES: failures, RUST_TARGET: target, INCLUDE_SCALAR: scalar,
            LIBGHOSTTY_VT_OPTIMIZE: "ReleaseFast", LIBGHOSTTY_VT_SIMD: "true" },
        });
        return { status: result.status, stderr: result.stderr, calls: readFileSync(log, "utf8").trim().split("\n") };
      };
      const recovery = run("aarch64-apple-darwin", "false", "2");
      expect(recovery.status).toBe(0);
      expect(recovery.calls).toHaveLength(3);
      expect(new Set(recovery.calls)).toEqual(new Set([
        "build --help -Demit-lib-vt -Dtarget=aarch64-macos -Dversion-string=1.3.2-dev -Demit-xcframework=false -Doptimize=ReleaseFast -Dsimd=true",
      ]));
      const failure = run("x86_64-pc-windows-msvc", "false", "3");
      expect(failure.status).toBe(1);
      expect(failure.calls).toHaveLength(3);
      expect(failure.calls.every((call) => call.includes("-Dtarget=x86_64-windows-msvc"))).toBe(true);
      expect(failure.stderr).toContain("attempt 3/3");
      const linux = run("", "true", "0");
      expect(linux.status).toBe(0);
      expect(linux.calls).toHaveLength(2);
      expect(linux.calls[0]).toContain("-Dtarget=x86_64-linux-gnu");
      expect(linux.calls[1]).toContain("-Doptimize=ReleaseSafe -Dsimd=false");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("only fork macOS jobs use the Intel capacity fallback", () => {
    const selection = "${{ github.repository == 'XinqiBao/herdr' && matrix.os == 'macos-latest' && 'macos-15-intel' || matrix.os }}";
    const ci = load("ci");
    expect(ci.jobs.check["runs-on"]).toBe(selection);
    expect(ci.jobs.check["timeout-minutes"]).toBe("${{ github.repository == 'XinqiBao/herdr' && matrix.os == 'macos-latest' && 30 || 15 }}");
    expect(build.jobs["build-macos"]["runs-on"]).toBe(selection);
    expect(ci.jobs.check.strategy.matrix.include.find((row: any) => row.os === "macos-latest")).toEqual({
      os: "macos-latest", kind: "unix", nextest_filter: "not binary(live_handoff)",
    });
    expect(build.jobs["build-macos"].strategy.matrix.include).toEqual([
      { target: "x86_64-apple-darwin", os: "macos-latest", name: "herdr-macos-x86_64" },
      { target: "aarch64-apple-darwin", os: "macos-latest", name: "herdr-macos-aarch64" },
    ]);
    expect(build.jobs["build-linux"]["runs-on"]).toBe("${{ matrix.os }}");
    expect(build.jobs["build-windows"]["runs-on"]).toBe("windows-latest");
  });

  test("only fork mainline dispatch can reach checks and all production targets", () => {
    expect(rolling.on).toEqual({ workflow_dispatch: null });
    expect(rolling.jobs.preflight.if).toBe("github.repository == 'XinqiBao/herdr' && github.ref == 'refs/heads/xbao/main'");
    for (const job of [rolling.jobs.checks, rolling.jobs.build]) {
      expect(job.needs).toBe("preflight");
      expect(job.if).toBeUndefined(); // implicit success(): failed/skipped preflight blocks both
    }
    expect(rolling.jobs.checks.uses).toBe("./.github/workflows/ci.yml");
    expect(rolling.jobs.build.with).toEqual({
      build_group: "all", libghostty_optimize: "ReleaseFast", libghostty_simd: "${{ 'true' }}",
    });
    expect(build.on.workflow_call.inputs.libghostty_simd.type).toBe("string");
    expect(build.jobs["build-linux"].strategy.matrix.include).toHaveLength(2);
    expect(build.jobs["build-macos"].strategy.matrix.include).toHaveLength(2);
    expect(build.jobs["build-windows"]["runs-on"]).toBe("windows-latest");
  });

  test("failed or skipped checks/build cannot run publication", () => {
    expect(rolling.jobs.publish.needs).toEqual(["checks", "build"]);
    expect(rolling.jobs.publish.if).toBeUndefined(); // retain GitHub's implicit success() gate
    expect(rolling.concurrency).toEqual({ group: "fork-rolling-release", "cancel-in-progress": false });
  });

  test("backup upload must succeed before the publisher can mutate anything", () => {
    const steps = rolling.jobs.publish.steps;
    const snapshot = steps.findIndex((s: any) => s.run === "python3 scripts/fork_publish.py snapshot");
    const backup = steps.findIndex((s: any) => s.id === "backup");
    const publish = steps.findIndex((s: any) => s.run === "python3 scripts/fork_publish.py publish");
    expect(snapshot).toBeGreaterThan(0);
    expect(backup).toBeGreaterThan(snapshot);
    expect(publish).toBeGreaterThan(backup);
    expect(steps[backup].uses).toMatch(/^actions\/upload-artifact@/);
    expect(steps[backup].with["if-no-files-found"]).toBe("error");
    expect(steps[backup]["continue-on-error"]).toBeUndefined();
    expect(steps[publish].if).toBeUndefined();
    expect(steps[publish].env.BACKUP_ARTIFACT_ID).toBe("${{ steps.backup.outputs.artifact-id }}");
    expect(steps.at(-1).if).toBe("always() && steps.backup.outcome == 'success'");
  });

  test("official side effects retain canonical repository guards", () => {
    expect(load("website-deploy").jobs.trigger.if).toBe("github.repository == 'herdrdev/herdr'");
    expect(load("label-next-release-issues").jobs.close.if).toBe("github.repository == 'herdrdev/herdr'");
  });
});
