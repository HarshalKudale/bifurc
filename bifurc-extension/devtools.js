/**
 * Bifurc Companion — DevTools bootstrap page.
 * Registers the Bifurc tab in Chrome DevTools.
 */
chrome.devtools.panels.create(
    "Bifurc",
    "icons/icon16.png",
    "devtools-panel.html"
);
