import { test, expect, Page } from "@playwright/test";
async function fixture(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (v: unknown) => void>();
    let n = 0;
    const listeners = new Map<string, number>();
    const agent = {
      mode: "official",
      library: {
        official_accounts: [
          {
            id: "example-account",
            name: "example 官方账号",
            official_auth: null,
          },
        ],
        custom_providers: [{ id: "example-api", name: "example API" }],
        selected_official: "example-account",
        selected_custom: "example-api",
        official_proxy_url: "",
        official_model: "",
        official_effort: "",
      },
      custom_fields: {
        "example-api": {
          provider_name: "example",
          base_url: "https://api.example.com/v1",
          api_key: "example-key",
          model: "example-model",
          effort: "medium",
          proxy_url: "",
        },
      },
    };
    let view = {
      revision: "example-revision",
      draft: {
        codex: { agent: "codex", ...structuredClone(agent) },
        claude: { agent: "claude", ...structuredClone(agent) },
      },
      roots: {
        acs: "/example/acs",
        codex: "/example/codex",
        claude: "/example/claude",
        claude_json: "/example/claude/.claude.json",
        claude_desktop: "/example/claude-desktop",
      },
    };
    const win = window as unknown as Record<string, any>;
    win.calls = [];
    win.__TAURI_INTERNALS__ = {
      transformCallback: (cb: (v: unknown) => void) => {
        callbacks.set(++n, cb);
        return n;
      },
      unregisterCallback: (id: number) => callbacks.delete(id),
      invoke: async (command: string, args: any) => {
        win.calls.push({ command, args });
        if (command === "plugin:event|listen") {
          listeners.set(args.event, args.handler);
          return ++n;
        }
        if (command === "plugin:event|unlisten") return;
        if (command === "load") return structuredClone(view);
        if (command === "save") {
          if (win.failSave) throw "example 保存失败";
          view = { ...view, draft: args.draft };
          return structuredClone(view);
        }
        if (command === "launch" && win.failLaunch)
          throw "example 客户端未安装";
        if (command === "repair") {
          callbacks.get(listeners.get("repair-progress")!)?.({
            event: "repair-progress",
            payload: { phase: "备份对话", completed: 1, total: 2 },
          });
          return { changed: 2, backup: "example-backup" };
        }
      },
    };
    win.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    win.requestNativeClose = () =>
      callbacks.get(listeners.get("request-close")!)?.({
        event: "request-close",
        payload: null,
      });
  });
  await page.goto("/");
  await expect(page.getByLabel("配置名称")).toHaveValue("example 官方账号");
}
const calls = (page: Page, command: string) =>
  page.evaluate(
    (c) => (window as any).calls.filter((x: any) => x.command === c).length,
    command,
  );

test("Claude edits stay in the Claude draft and survive switching clients", async ({ page }) => {
  await fixture(page);
  await page.getByRole("tab", { name: "Claude", exact: true }).click();
  await expect(page.getByText("配置未修改", { exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "自定义 API", exact: true }).click();
  await page.getByLabel("API 地址", { exact: true }).fill("https://claude.example.com");
  await page.getByLabel("HTTP 代理（可留空）", { exact: true }).fill("http://127.0.0.1:7890");
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("已保存");
  await expect(page.getByLabel("API 地址", { exact: true })).toHaveValue("https://claude.example.com");
  await expect(page.getByText("配置未修改", { exact: true })).toBeVisible();
  const payload = await page.evaluate(() => (window as any).calls.find((c: any) => c.command === "save").args.draft);
  expect(payload.codex.custom_fields["example-api"].base_url).toBe("https://api.example.com/v1");
  expect(payload.claude.custom_fields["example-api"].base_url).toBe("https://claude.example.com");
  expect(payload.claude.custom_fields["example-api"].proxy_url).toBe("http://127.0.0.1:7890");
  expect(payload.codex.custom_fields["example-api"].proxy_url).toBe("");
  await page.getByRole("button", { name: "启动", exact: true }).click();
  expect(await calls(page, "launch")).toBe(1);
  await page.getByRole("tab", { name: "Codex", exact: true }).click();
  // 客户端各自记住自己的模式：Codex 仍是官方账号，Claude 的自定义 API 不受影响。
  await expect(page.getByRole("tab", { name: "官方账号", exact: true })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByText("配置未修改", { exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "自定义 API", exact: true }).click();
  await expect(page.getByLabel("API 地址", { exact: true })).toHaveValue("https://api.example.com/v1");
  await page.getByRole("tab", { name: "Claude", exact: true }).click();
  await expect(page.getByLabel("API 地址", { exact: true })).toHaveValue("https://claude.example.com");
});

test("switching clients keeps both drafts and reports unsaved changes", async ({
  page,
}) => {
  await fixture(page);
  // 在一个客户端里编辑，再切到另一个：草稿保留，状态仍显示未保存。
  await page.getByLabel("配置名称").fill("example codex draft");
  await page.getByRole("tab", { name: "Claude", exact: true }).click();
  await expect(page.getByText("有未保存修改", { exact: true })).toBeVisible();
  await page.getByLabel("配置名称").fill("example claude draft");
  await page.getByRole("tab", { name: "Codex", exact: true }).click();
  await expect(page.getByLabel("配置名称")).toHaveValue("example codex draft");
  await page.getByRole("button", { name: "启动", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("未保存修改");
  expect(await calls(page, "launch")).toBe(0);
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("已保存");
  const payload = await page.evaluate(() => (window as any).calls.find((c: any) => c.command === "save").args);
  expect(payload.agent).toBe("codex");
  expect(payload.draft.codex.library.official_accounts[0].name).toBe("example codex draft");
  expect(payload.draft.claude.library.official_accounts[0].name).toBe("example claude draft");
});

test("opens settings; save and keyboard never start client; dirty launch blocked", async ({
  page,
}) => {
  await fixture(page);
  await page.getByLabel("配置名称").fill("example edited");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Space");
  expect(await calls(page, "launch")).toBe(0);
  expect(await calls(page, "save")).toBe(0);
  await page.getByRole("button", { name: "启动", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("未保存修改");
  expect(await calls(page, "launch")).toBe(0);
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("已保存");
  expect(await calls(page, "launch")).toBe(0);
  expect(await calls(page, "close")).toBe(0);
  await page.getByRole("button", { name: "启动", exact: true }).click();
  expect(await calls(page, "launch")).toBe(1);
});
test("close button, Escape and native titlebar share discard confirmation", async ({
  page,
}) => {
  await fixture(page);
  await page.getByLabel("配置名称").fill("example changed");
  for (const route of ["button", "escape", "native"]) {
    if (route === "button")
      await page.getByRole("button", { name: "关闭", exact: true }).click();
    if (route === "escape") await page.keyboard.press("Escape");
    if (route === "native")
      await page.evaluate(() => (window as any).requestNativeClose());
    await expect(page.getByRole("alertdialog")).toBeVisible();
    await page.getByRole("button", { name: "继续编辑" }).click();
    await expect(page.getByRole("alertdialog")).toBeHidden();
    expect(await calls(page, "close")).toBe(0);
  }
  await page.getByRole("button", { name: "关闭", exact: true }).click();
  await page.getByRole("button", { name: "放弃修改并关闭" }).click();
  expect(await calls(page, "close")).toBe(1);
});
test("failed save retains draft and failed launch stays open for retry", async ({
  page,
}) => {
  await fixture(page);
  await page.evaluate(() => ((window as any).failSave = true));
  await page.getByLabel("配置名称").fill("example retained");
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("保存失败");
  await expect(page.getByLabel("配置名称")).toHaveValue("example retained");
  await page.evaluate(() => {
    (window as any).failSave = false;
    (window as any).failLaunch = true;
  });
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await page.getByRole("button", { name: "启动", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("客户端未安装");
  expect(await calls(page, "close")).toBe(0);
  await page.evaluate(() => ((window as any).failLaunch = false));
  await page.getByRole("button", { name: "启动", exact: true }).click();
  expect(await calls(page, "launch")).toBe(2);
});
test("mode and profile edits stay drafts; history repair is explicit", async ({
  page,
}) => {
  await fixture(page);
  await page.getByRole("tab", { name: "自定义 API", exact: true }).click();
  await page.getByRole("button", { name: "复制", exact: true }).click();
  await expect(page.getByLabel("配置名称")).toHaveValue("example API 副本");
  expect(await calls(page, "save")).toBe(0);
  await page.getByRole("button", { name: "修复对话", exact: true }).click();
  await page.getByRole("button", { name: "继续编辑" }).click();
  expect(await calls(page, "repair")).toBe(0);
  await page.getByRole("button", { name: "修复对话", exact: true }).click();
  await page.getByRole("button", { name: "备份并修复" }).click();
  await expect(page.getByRole("status")).toContainText("已修复 2 项");
  expect(await calls(page, "save")).toBe(0);
});
test("settings fit desktop, compact window and large text in both themes", async ({
  page,
}) => {
  await fixture(page);
  for (const appearance of ["light", "dark"] as const) {
    await page.emulateMedia({ reducedMotion: "reduce" });
    if (appearance === "dark") {
      await page.getByRole("button", { name: "深色", exact: true }).focus();
      await page.keyboard.press("Space");
      await expect(page.getByRole("button", { name: "浅色", exact: true })).toBeVisible();
    }
    for (const [width, height] of [[700, 720], [600, 620], [1000, 800]]) {
      await page.setViewportSize({ width, height });
      for (const mode of ["官方账号", "自定义 API"]) {
        await page.getByRole("tab", { name: mode, exact: true }).click();
        await expect(page.getByRole("tab", { name: mode, exact: true })).toHaveAttribute("aria-selected", "true");
        await expect(page.getByRole("button", { name: "保存配置", exact: true })).toBeVisible();
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        const formBounds = await page.locator(".form").boundingBox();
        expect(Math.round(formBounds!.width)).toBe(Math.min(width <= 650 ? width - 36 : width - 56, 720));
        expect(Math.abs(formBounds!.x + formBounds!.width / 2 - width / 2)).toBeLessThan(1);
        // 默认窗口高度下两种模式都必须完整显示，不出现纵向滚动。
        if (width === 700)
          expect(await page.evaluate(() => document.documentElement.scrollHeight <= innerHeight)).toBe(true);
        await page.screenshot({
          path: `.impeccable/review/${width}-${mode}${appearance === "dark" ? "-dark" : ""}.png`,
          fullPage: true,
          animations: "disabled",
        });
      }
    }
    // Contrast uses the browser's actual resolved theme, including control overrides.
    const ratios = await page.evaluate(() => {
      const luminance = (color: string) => {
        const channels = color.match(/[\d.]+/g)!.slice(0, 3).map(Number).map(v => {
          v /= 255;
          return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
        });
        return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
      };
      return ["header p", ".primary-action", ".launch-action", ".mode-switch [aria-selected=true]"].map(selector => {
        const element = document.querySelector(selector)!;
        const style = getComputedStyle(element);
        const background = selector === "header p" ? getComputedStyle(document.querySelector(".app")!).backgroundColor
          : selector.includes("mode-switch") ? getComputedStyle(document.querySelector(".mode-switch")!, "::before").backgroundColor
          : style.backgroundColor;
        const a = luminance(style.color), b = luminance(background);
        return { selector, ratio: (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05) };
      });
    });
    for (const { selector, ratio } of ratios) expect(ratio, selector).toBeGreaterThanOrEqual(4.5);
  }
  expect(await calls(page, "save")).toBe(0);
  expect(await calls(page, "launch")).toBe(0);
  await page.getByRole("tab", { name: "官方账号", exact: true }).focus();
  await page.keyboard.press("ArrowRight");
  await expect(page.getByRole("tab", { name: "自定义 API", exact: true })).toBeFocused();
  await page.evaluate(() => (document.documentElement.style.fontSize = "24px"));
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});
