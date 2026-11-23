const copyStatus = document.querySelector("#copy-status");

if (navigator.clipboard?.writeText) {
  for (const button of document.querySelectorAll(".copy")) {
    const command = button.previousElementSibling;
    let resetLabel;
    button.hidden = false;

    button.addEventListener("click", async () => {
      clearTimeout(resetLabel);
      try {
        await navigator.clipboard.writeText(command.textContent);
        button.textContent = "Copied";
        copyStatus.textContent = "Command copied to the clipboard.";
      } catch {
        const range = document.createRange();
        range.selectNodeContents(command);
        const selection = window.getSelection();
        selection.removeAllRanges();
        selection.addRange(range);
        button.textContent = "Selected";
        copyStatus.textContent = "Command selected for manual copying.";
      }
      resetLabel = setTimeout(() => {
        button.textContent = "Copy";
        copyStatus.textContent = "";
      }, 1800);
    });
  }
}
