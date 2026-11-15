import { chromium } from "playwright-core";

const [url, mode = "todo-ui"] = process.argv.slice(2);
const steps = [];
let ok = true;

function record(name, passed, detail = "") {
  steps.push({ name, ok: Boolean(passed), detail: String(detail || "") });
  if (!passed) ok = false;
}

function withTimeout(promise, ms, fallback) {
  return Promise.race([
    promise,
    new Promise((resolve) => setTimeout(() => resolve(fallback), ms)),
  ]);
}

async function isVisible(locator, timeout = 7000) {
  try {
    await locator.first().waitFor({ state: "visible", timeout });
    return true;
  } catch {
    return false;
  }
}

async function isHidden(locator, timeout = 7000) {
  try {
    await locator.first().waitFor({ state: "hidden", timeout });
    return true;
  } catch {
    return false;
  }
}

function textLocator(page, text) {
  return page.getByText(text, { exact: false });
}

async function clickFilter(page, name) {
  const exact = new RegExp(`^${name}(\\s*\\(\\d+\\))?$`, "i");
  for (const role of ["button", "link", "tab"]) {
    const candidate = page.getByRole(role, { name: exact });
    if (await candidate.count()) {
      await candidate.first().click();
      return true;
    }
  }
  const loose = page.getByText(exact);
  if (await loose.count()) {
    await loose.first().click();
    return true;
  }
  // Last resort: a button whose label contains the word.
  const contains = page.getByRole("button", { name: new RegExp(name, "i") });
  if (await contains.count()) {
    await contains.first().click();
    return true;
  }
  return false;
}

async function clickClearCompleted(page) {
  const exact = page.getByRole("button", {
    name: /clear completed|clear done|remove completed|delete completed/i,
  });
  if (await exact.count()) {
    await exact.first().click();
    return true;
  }
  const loose = page.getByText(/clear completed|clear done|remove completed/i);
  if (await loose.count()) {
    await loose.first().click();
    return true;
  }
  return false;
}

async function deleteVisibleItem(page, text) {
  const item = textLocator(page, text).first();
  let container = page.locator("li", { hasText: text }).first();
  if (!(await container.count())) {
    container = item.locator("xpath=ancestor::*[self::li or self::article or self::div][1]");
  }
  const button = container
    .getByRole("button", { name: /delete|remove|trash|×|✕/i })
    .first();
  if (await button.count()) {
    await button.click();
    return true;
  }
  const anyButton = container.locator("button").first();
  if (await anyButton.count()) {
    await anyButton.click();
    return true;
  }
  return false;
}

async function main() {
  let browser;
  try {
    browser = await chromium.launch({ channel: "chrome", headless: true });
  } catch (error) {
    try {
      browser = await chromium.launch({ channel: "chromium", headless: true });
    } catch {
      if (!process.env.CHROME_PATH) throw error;
      browser = await chromium.launch({
        executablePath: process.env.CHROME_PATH,
        headless: true,
      });
    }
  }

  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  page.setDefaultTimeout(8000);
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(String(error)));
  page.on("dialog", (dialog) => dialog.dismiss().catch(() => {}));

  await page.goto(url, { waitUntil: "domcontentloaded", timeout: 30000 });
  await page.waitForTimeout(500);

  const input = page
    .getByRole("textbox")
    .first()
    .or(page.locator('input[type="text"], input:not([type])').first());
  const addButton = page
    .getByRole("button", { name: /add|create|save|submit/i })
    .first();
  const hasInput = await isVisible(input);
  const hasAdd = await addButton.count();
  record("input and add control render", hasInput && hasAdd > 0, `input=${hasInput} addButtons=${hasAdd}`);

  if (!hasInput) {
    console.log("BENCH_RESULT " + JSON.stringify({ ok: false, steps, mode }));
    await browser.close();
    process.exitCode = 1;
    return;
  }

  async function addTodo(text, useButton) {
    await input.fill(text);
    if (useButton && (await addButton.count())) {
      await addButton.first().click();
    } else {
      await input.press("Enter");
    }
    const added = await isVisible(textLocator(page, text));
    if (!added && (await addButton.count())) {
      await addButton.first().click();
    }
    return isVisible(textLocator(page, text));
  }

  record("add todo with Enter", await addTodo("Buy milk", false));
  record("add todo with button", await addTodo("Walk dog", true));

  const milk = textLocator(page, "Buy milk").first();
  let milkContainer = page.locator("li", { hasText: "Buy milk" }).first();
  let checkbox = milkContainer.locator('input[type="checkbox"]').first();
  if (!(await checkbox.count())) {
    checkbox = page.locator('input[type="checkbox"]').first();
  }
  let toggled = false;
  if (await checkbox.count()) {
    await checkbox.check().catch(async () => {
      await milk.click().catch(() => {});
    });
    toggled = await checkbox.isChecked().catch(() => true);
  } else {
    await milk.click().catch(() => {});
    toggled = true;
  }
  record("toggle a todo complete", toggled);

  const activeClicked = await clickFilter(page, "Active");
  const activeOk =
    activeClicked &&
    (await isHidden(textLocator(page, "Buy milk"))) &&
    (await isVisible(textLocator(page, "Walk dog")));
  record("active filter hides completed", activeOk, `clicked=${activeClicked}`);

  const completedClicked = await clickFilter(page, "Completed");
  const completedOk =
    completedClicked &&
    (await isVisible(textLocator(page, "Buy milk"))) &&
    (await isHidden(textLocator(page, "Walk dog")));
  record("completed filter hides active", completedOk, `clicked=${completedClicked}`);

  const allClicked = await clickFilter(page, "All");
  const allOk =
    allClicked &&
    (await isVisible(textLocator(page, "Buy milk"))) &&
    (await isVisible(textLocator(page, "Walk dog")));
  record("all filter shows everything", allOk, `clicked=${allClicked}`);

  const cleared = await clickClearCompleted(page);
  const clearOk = cleared && (await isHidden(textLocator(page, "Buy milk")));
  record("clear completed removes completed todos", clearOk, `clicked=${cleared}`);

  const deleted = await deleteVisibleItem(page, "Walk dog");
  const deleteOk = deleted && (await isHidden(textLocator(page, "Walk dog")));
  record("delete removes a todo", deleteOk, `clicked=${deleted}`);

  record("add todo before reload", await addTodo("Persist me", false));
  await page.reload({ waitUntil: "domcontentloaded" });
  await page.waitForTimeout(500);
  const persisted = await isVisible(textLocator(page, "Persist me"));
  record("localStorage persists across reload", persisted);

  record(
    "no uncaught page errors",
    pageErrors.length === 0,
    pageErrors.slice(0, 3).join(" | "),
  );

  await browser.close();
  console.log("BENCH_RESULT " + JSON.stringify({ ok, mode, steps }));
  process.exitCode = ok ? 0 : 1;
}

main().catch(async (error) => {
  console.log(
    "BENCH_RESULT " +
      JSON.stringify({ ok: false, mode, steps, error: String(error) }),
  );
  process.exitCode = 1;
});
