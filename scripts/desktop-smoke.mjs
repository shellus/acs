import { chromium } from '@playwright/test';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, writeFile, mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import assert from 'node:assert/strict';

// The production binary runs against an isolated fixture; no test command is compiled into it.
const fixture = await mkdtemp(join(tmpdir(), 'acs-example-'));
const port = 19227;
const executable = resolve(process.argv[2] ?? 'target/release/acs-gui.exe');
const windowState = join(fixture, 'acs', 'window.json');
let child, browser;

async function open() {
  child = spawn(executable, [], { windowsHide: true, env: {
    ...process.env,
    ACS_HOME: join(fixture, 'acs'),
    CODEX_HOME: join(fixture, 'codex'),
    CLAUDE_CONFIG_DIR: join(fixture, 'claude'),
    ACS_CLAUDE_DESKTOP_DIR: join(fixture, 'claude-desktop'),
    WEBVIEW2_USER_DATA_FOLDER: join(fixture, 'webview'),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
  } });
  for (let attempt = 0; attempt < 120; attempt++) {
    if (child.exitCode !== null) throw new Error(`Desktop exited with ${child.exitCode}`);
    try {
      browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 1000 });
      break;
    } catch {}
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  if (!browser) throw new Error('Desktop debugging connection did not become ready');
  const context = browser.contexts()[0];
  const page = context.pages()[0] ?? await context.waitForEvent('page');
  await page.getByRole('tab', { name: '官方账号', exact: true }).waitFor();
  const view = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('load'));
  for (const name of ['acs', 'codex', 'claude', 'claude-desktop']) {
    assert.equal(resolve(view.roots[name.replace('-', '_')]), join(fixture, name), `Refusing writes outside isolated ${name} fixture`);
  }
  assert.equal(resolve(view.roots.claude_json), join(fixture, 'claude', '.claude.json'));
  return page;
}
// A production app may exit before WebView acknowledges the click. Verify the
// actual process exit instead of treating that expected disconnect as failure.
async function exits(action) {
  const running = child;
  let timer;
  const exited = new Promise((resolve, reject) => {
    running.once('exit', (code, signal) => { clearTimeout(timer); resolve({code, signal}); });
    timer = setTimeout(() => reject(new Error('Desktop did not exit after confirmation')), 10000);
  });
  await action().catch(error => {
    if (!String(error).includes('Target page, context or browser has been closed')) throw error;
  });
  assert.deepEqual(await exited, {code: 0, signal: null});
  await browser.close();
  browser = undefined;
}
// DPI 缩放会让逻辑像素在往返换算后出现个位数误差，尺寸断言按近似值比较。
function near(actual, expected, what) {
  assert.ok(actual.every((v, i) => Math.abs(v - expected[i]) <= 2), `${what}：期望 ${expected}，实际 ${actual}`);
}
// WebView 先以初始尺寸完成首帧，还原后的尺寸稍后才生效，按结果轮询而不是立即读取。
async function sized(page, expected, what) {
  await page.waitForFunction(
    ([width, height]) => Math.abs(innerWidth - width) <= 2 && Math.abs(innerHeight - height) <= 2,
    expected, { timeout: 10000 },
  ).catch(async () => near(await page.evaluate(() => [innerWidth, innerHeight]), expected, what));
}

try {
  const page = await open();
  await page.getByRole('tab', { name: '自定义 API', exact: true }).click();
  await page.getByRole('button', { name: '添加', exact: true }).click();
  for (const [label, value] of Object.entries({ '配置名称': 'example API', '提供者显示名称': 'example provider', 'API 地址': 'https://api.example.com/v1', 'API Key': 'example-key', '模型': 'example-model' })) {
    await page.getByLabel(label, {exact: true}).fill(value);
  }
  await page.getByRole('button', {name:'启动',exact:true}).click();
  await page.getByRole('alert').filter({hasText:'未保存修改'}).waitFor();
  await page.getByRole('button', {name:'保存配置',exact:true}).click();
  await page.getByRole('status').filter({hasText:'已保存'}).waitFor();
  assert.match(await readFile(join(fixture,'codex','config.toml'),'utf8'), /model_provider = "custom"/);
  assert.equal(JSON.parse(await readFile(join(fixture,'codex','auth.json'),'utf8')).OPENAI_API_KEY,'example-key');
  await mkdir('.impeccable/review',{recursive:true});
  await page.screenshot({path:'.impeccable/review/native-windows.png'});
  // Claude 的无 /v1 地址必须走自己的校验，不能覆盖 Codex 兼容字段。
  const codexConfig = await readFile(join(fixture, 'codex', 'config.toml'), 'utf8');
  const codexAuth = await readFile(join(fixture, 'codex', 'auth.json'), 'utf8');
  await page.getByRole('tab', {name:'Claude',exact:true}).click();
  await page.getByText('配置未修改', {exact:true}).waitFor();
  await page.getByRole('tab', {name:'自定义 API',exact:true}).click();
  await page.getByRole('button', {name:'添加',exact:true}).click();
  for (const [label, value] of Object.entries({'配置名称':'example Claude', 'API 地址':'https://claude.example.com', 'API Key':'example-claude-key', '模型':'example-claude-model'})) {
    await page.getByLabel(label, {exact:true}).fill(value);
  }
  await page.getByRole('button', {name:'保存配置',exact:true}).click();
  await page.getByRole('status').filter({hasText:'已保存'}).waitFor();
  assert.equal(await page.getByLabel('API 地址', {exact:true}).inputValue(), 'https://claude.example.com');
  await page.getByText('配置未修改', {exact:true}).waitFor();
  const claudeSettings = JSON.parse(await readFile(join(fixture, 'claude', 'settings.json'), 'utf8'));
  assert.equal(claudeSettings.env.ANTHROPIC_BASE_URL, 'https://claude.example.com');
  assert.equal(claudeSettings.env.ANTHROPIC_AUTH_TOKEN, 'example-claude-key');
  const desktopProfile = JSON.parse(await readFile(join(fixture, 'claude-desktop', 'configLibrary', '00000000-0000-4000-8000-000000000ac5.json'), 'utf8'));
  assert.equal(desktopProfile.inferenceGatewayBaseUrl, 'https://claude.example.com');
  assert.equal(JSON.parse(await readFile(join(fixture, 'claude-desktop', 'claude_desktop_config.json'), 'utf8')).deploymentMode, '3p');
  assert.equal(await readFile(join(fixture, 'codex', 'config.toml'), 'utf8'), codexConfig);
  assert.equal(await readFile(join(fixture, 'codex', 'auth.json'), 'utf8'), codexAuth);
  await page.getByRole('tab', {name:'Codex',exact:true}).click();
  assert.equal(await page.getByLabel('API 地址', {exact:true}).inputValue(), 'https://api.example.com/v1');
  await page.getByText('配置未修改', {exact:true}).waitFor();
  await writeFile(join(fixture,'codex','.env'),'EXAMPLE=external\n');
  await page.getByLabel('配置名称').fill('example retained draft');
  await page.getByRole('button', {name:'保存配置',exact:true}).click();
  await page.getByRole('alert').filter({hasText:'其他程序修改'}).waitFor();
  assert.equal(await page.getByLabel('配置名称').inputValue(),'example retained draft');
  await page.getByRole('button', {name:'关闭',exact:true}).click();
  await page.getByRole('button', {name:'继续编辑',exact:true}).click();
  await page.getByRole('alertdialog').waitFor({state:'hidden'});
  await page.keyboard.press('Escape');
  await exits(() => page.getByRole('button', {name:'放弃修改并关闭',exact:true}).click());

  // 退出时记住窗口几何，下次打开按记住的尺寸还原，而不是回到配置里的默认值。
  const saved = JSON.parse(await readFile(windowState,'utf8'));
  near([saved.width, saved.height], [700, 720], '退出未记录默认窗口尺寸');
  await writeFile(windowState, JSON.stringify({...saved, width: 900, height: 660}));
  const restored = await open();
  await sized(restored, [900, 660], '窗口未按记住的尺寸还原');
  await exits(() => restored.getByRole('button', {name:'关闭',exact:true}).click());
  const reread = JSON.parse(await readFile(windowState,'utf8'));
  near([reread.width, reread.height], [900, 660], '再次退出覆盖了记住的尺寸');

  console.log('PASS: production Tauri window, actual IPC, isolated save, dirty/external conflict, close cancellation, Escape and window geometry persistence');
  console.log(`Fixture: ${fixture}`);
} finally {
  await browser?.close();
  if (child?.exitCode === null) child.kill();
}
