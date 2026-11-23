// HD-05: each route renders on the real binary, logs no error and passes an axe scan, in every browser.
import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "./fixtures";
import { ROUTES, WCAG } from "./routes";

for (const r of ROUTES) {
  test(`${r.path} renders and passes axe`, async ({ context, go, browserName }) => {
    // Dates are written by lib/date.ts, not the runtime's locale, so the HTML Node renders matches every browser's.
    const errors: string[] = [];
    // Known, not ours: Firefox 1572's content process sometimes crashes while axe's script is evaluated (about one
    // scan in ten, on any route, with or without load). A crashed scan is run once more on a fresh page.
    let page = context.pages()[0];
    const scan = async () => {
      page.on("pageerror", (e) => errors.push(e.message));
      await go(r.path, page);
      await expect(r.ready(page)).toBeVisible();
      return new AxeBuilder({ page }).withTags(WCAG).analyze();
    };
    const { violations } = await scan().catch(async (e: Error) => {
      if (browserName !== "firefox" || !/Page crashed/.test(e.message)) throw e;
      test.info().annotations.push({ type: "retried", description: "Firefox crashed during the axe scan" });
      page = await context.newPage();
      return scan();
    });
    expect(violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
    expect(errors).toEqual([]);
  });
}
