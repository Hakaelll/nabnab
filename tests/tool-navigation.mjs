export async function openTool(page, id) {
  const directoryEntry = page.locator(`#local-tool-${id}`);
  if (await directoryEntry.isVisible()) {
    await directoryEntry.click();
    return;
  }
  if (!await page.locator(".tool-catalog").isVisible()) {
    await page.locator('[data-action="tool-catalog-open"]').click();
  }
  const operation = page.locator(`.tool-catalog-operation[data-id="${id}"]`);
  const group = await operation.getAttribute("data-catalog-group");
  await page.locator(`.tool-catalog-item[data-id="${group}"]`).click();
  await operation.click();
}
