const syntax = {
  latex: /(?<comment>%[^\r\n]*)|(?<command>\\(?:[\p{L}@:_]+|[^\r\n]))|(?<symbol>[{}[\]&$^_])/gu,
  bibtex: /(?<comment>^[ \t]*%[^\r\n]*)|(?<command>@[a-z]+)|(?<field>(?<![\w-])[a-z][\w-]*(?=[ \t]*=))|(?<value>"(?:\\[\s\S]|[^"\\])*"|\b\d+\b)|(?<symbol>[{}(),=#])/gim,
};

function highlight(code, text, language) {
  const fragment = document.createDocumentFragment();
  let offset = 0;
  for (const token of text.matchAll(syntax[language])) {
    fragment.append(text.slice(offset, token.index));
    const span = document.createElement("span");
    const kind = Object.keys(token.groups).find((name) => token.groups[name] !== undefined);
    span.className = "syntax-" + kind;
    span.textContent = token[0];
    fragment.append(span);
    offset = token.index + token[0].length;
  }
  fragment.append(text.slice(offset));
  code.replaceChildren(fragment);
}

function bindCopy(button, code, status, label) {
  let resetLabel;
  button.addEventListener("click", async () => {
    clearTimeout(resetLabel);
    const text = code.textContent;
    let copied = false;
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
    } catch {
      // Allow manual copying when clipboard access is unavailable.
    }
    if (button.disabled || text !== code.textContent) {
      return;
    }
    if (!copied) {
      const range = document.createRange();
      range.selectNodeContents(code);
      const selection = window.getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
    }
    button.textContent = copied ? "Copied" : "Selected";
    status.textContent = copied
      ? label + " copied to the clipboard."
      : label + " selected for manual copying.";
    resetLabel = setTimeout(() => {
      button.textContent = "Copy";
      status.textContent = "";
    }, 1800);
  });
}

function setupPlayground() {
  const source = document.querySelector("#playground-source");
  const highlighted = document.querySelector("#source-highlight");
  const output = document.querySelector("#playground-output");
  const status = document.querySelector("#playground-status");
  const copy = document.querySelector("#copy-output");
  const languages = document.querySelector(".language-switch");
  const examples = {
    latex: [
      String.raw`% Align equations and their labels
\begin{align}
v&=v_0+at\label{eq:velocity}\\
x(t)&=x_0+v_0t+\frac12 at^2\label{eq:position}
\end{align}
`,
      String.raw`% Line up table columns
\begin{tabular}{lrr}
Method&Trials&Accuracy\\
\hline
Baseline&20&0.81\\
Fine-tuned&120&0.94\\
\end{tabular}
`,
      String.raw`% Indent a nested list
\begin{enumerate}
\item Prepare the data.
\begin{itemize}
\item Remove duplicates.
\item Split the training and test sets.
\end{itemize}
\item Fit the model.
\end{enumerate}
`,
      String.raw`% Lay out a figure and its caption
\begin{figure}[htbp]
\centering
\includegraphics[width=.8\linewidth]{results}
\caption{Accuracy by training epoch.}
\label{fig:accuracy}
\end{figure}
`,
      String.raw`% Align a matrix
\[
A=\begin{pmatrix}
1&-12&0\\
100&3&24\\
0&7&-1
\end{pmatrix}
\]
`,
    ],
    bibtex: [
      `% Align journal article fields
@article{shannon1948,author={Claude E. Shannon},
title={A Mathematical Theory of Communication},
journal={Bell System Technical Journal},volume={27},year={1948}}
`,
      `% Reuse a publisher across book entries
@string{aw="Addison-Wesley"}
@book{knuth1984,title={The {TeXbook}},author={Donald E. Knuth},
publisher=aw,year={1984}}
@book{lamport1994,title={{LaTeX}: A Document Preparation System},
author={Leslie Lamport},publisher=aw,year={1994}}
`,
      `% Keep capitalization in a conference reference
@inproceedings{attention2017,
author={Vaswani, Ashish and others}, title={Attention Is All You Need},
booktitle={Advances in Neural Information Processing Systems ({NeurIPS})},
pages={5998--6008},year={2017}}
`,
    ],
  };
  const snippets = [...examples.latex, ...examples.bibtex];
  let previous;
  try {
    previous = sessionStorage.getItem("tift-playground-example");
  } catch {
    // Storage can be disabled in a private browsing session.
  }
  const choices = snippets.filter(snippet => snippet !== previous);
  const pick = items => items[Math.floor(Math.random() * items.length)];
  const initial = pick(choices);
  let language = examples.latex.includes(initial) ? "latex" : "bibtex";
  try {
    sessionStorage.setItem("tift-playground-example", initial);
  } catch {
    // Random selection also works without storage.
  }
  const drafts = {
    latex: language === "latex" ? initial : pick(examples.latex),
    bibtex: language === "bibtex" ? initial : pick(examples.bibtex),
  };
  languages.querySelector(`input[value="${language}"]`).checked = true;
  let revision = 0;
  let timer;
  let worker;
  let ready = false;
  let composing = false;

  function setStatus(message, state = "") {
    if (status.textContent !== message) {
      status.textContent = message;
    }
    status.dataset.state = state;
  }

  function syncScroll() {
    highlighted.parentElement.scrollTop = source.scrollTop;
    highlighted.parentElement.scrollLeft = source.scrollLeft;
  }

  function update() {
    const text = source.value;
    const current = ++revision;
    highlight(highlighted, text, language);
    syncScroll();
    clearTimeout(timer);
    copy.disabled = true;
    copy.textContent = "Copy";

    if (!ready) {
      return;
    }
    if (!text.trim()) {
      output.replaceChildren();
      setStatus("Paste or type code to see formatted output.");
      return;
    }
    setStatus("Formatting…");
    if (!composing) {
      timer = setTimeout(() => worker.postMessage({
        revision: current,
        source: text,
        bibtex: language === "bibtex",
      }), 150);
    }
  }

  function formatterUnavailable() {
    ready = false;
    clearTimeout(timer);
    worker?.terminate();
    output.replaceChildren();
    copy.disabled = true;
    setStatus("The formatter could not load. Reload the page to try again.", "error");
  }

  bindCopy(copy, output, document.querySelector("#playground-copy-status"), "Formatted output");
  source.value = drafts[language];
  source.classList.add("highlighted");
  update();
  source.addEventListener("scroll", syncScroll, { passive: true });
  source.addEventListener("input", update);
  source.addEventListener("compositionstart", () => { composing = true; });
  source.addEventListener("compositionend", () => {
    composing = false;
    update();
  });
  languages.addEventListener("change", ({ target }) => {
    drafts[language] = source.value;
    language = target.value;
    source.value = drafts[language];
    source.scrollTo(0, 0);
    output.replaceChildren();
    update();
  });

  setStatus("Loading formatter…");
  try {
    worker = new Worker(import.meta.url, { type: "module" });
  } catch {
    formatterUnavailable();
    return;
  }
  worker.addEventListener("error", formatterUnavailable);
  worker.addEventListener("message", ({ data }) => {
    if (data.type === "unavailable") {
      formatterUnavailable();
    } else if (data.type === "ready") {
      ready = true;
      update();
    } else if (data.revision === revision) {
      if (data.hasError) {
        output.replaceChildren();
        setStatus(data.diagnostic, "error");
      } else {
        highlight(output, data.text, language);
        copy.disabled = data.text.length === 0;
        setStatus(data.diagnostic || "Formatted with default settings.", data.diagnostic ? "warning" : "");
      }
    }
  });
}

async function startFormatter() {
  let formatSnippet;
  try {
    const formatter = await import("./tift_playground.js");
    await formatter.default();
    formatSnippet = formatter.format_snippet;
  } catch {
    self.postMessage({ type: "unavailable" });
    return;
  }

  self.addEventListener("message", ({ data }) => {
    let result;
    try {
      result = formatSnippet(data.source, data.bibtex);
      self.postMessage({
        revision: data.revision,
        text: result.text,
        diagnostic: result.diagnostic,
        hasError: result.has_error,
      });
    } catch (error) {
      self.postMessage({
        revision: data.revision,
        diagnostic: "Formatting failed. " + error.message,
        hasError: true,
      });
    } finally {
      result?.free();
    }
  });
  self.postMessage({ type: "ready" });
}

if (typeof document === "undefined") {
  startFormatter();
} else {
  setupPlayground();
  if (navigator.clipboard?.writeText) {
    const status = document.querySelector("#copy-status");
    for (const button of document.querySelectorAll(".command .copy")) {
      bindCopy(button, button.previousElementSibling, status, "Command");
      button.hidden = false;
    }
  }
}
