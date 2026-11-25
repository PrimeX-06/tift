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
  const indentation = document.querySelector("#playground-indent");
  const examples = {
    latex: [
      String.raw`% Motion under constant acceleration
\section{A ball in flight}
Let $v_0$ be the launch speed and $\theta$ the angle.
Choose the launch point as the origin.

\begin{align}
a_x&=0, & a_y&=-g,\\
v_x(t)&=v_0\cos\theta, & v_y(t)&=v_0\sin\theta-gt,\\
x(t)&=v_0t\cos\theta, & y(t)&=v_0t\sin\theta-\tfrac12 gt^2.
\end{align}

At the highest point the vertical velocity is zero.
For a landing at the launch height we obtain
\begin{align}
t_{\mathrm{peak}}&=\frac{v_0\sin\theta}{g},\\
h_{\max}&=\frac{v_0^2\sin^2\theta}{2g},\\
t_{\mathrm{flight}}&=2t_{\mathrm{peak}},\\
R&=v_0t_{\mathrm{flight}}\cos\theta
\label{eq:range}
\end{align}
Equation~\eqref{eq:range} gives the horizontal range.
`,
      String.raw`% Compare classifiers on held-out data
\begin{table}[htbp]
\centering
\caption{Test results by classifier.}
\label{tab:classifiers}
\begin{tabular}{lrrr}
\hline
Model&Precision&Recall&F1\\
\hline
Majority vote&0.50&1.00&0.67\\
Naive Bayes&0.78&0.72&0.75\\
Logistic regression&0.84&0.80&0.82\\
Decision tree&0.81&0.79&0.80\\
Random forest&0.88&0.85&0.86\\
Gradient boosting&0.90&0.87&0.88\\
\hline
\end{tabular}
\end{table}

Table~\ref{tab:classifiers} compares the same test split.
Gradient boosting gives the highest F1 score.
`,
      String.raw`% Outline a reproducible experiment
\section{Experimental protocol}
\begin{enumerate}
\item Prepare the observations.
\begin{itemize}
\item Remove duplicate measurements.
\item Keep one row per subject.
\item Reserve a held-out test set.
\end{itemize}
\item Train the models.
\begin{itemize}
\item Fit a baseline on the training split.
\item Tune the learning rate on the validation split.
\item Save the selected parameters.
\end{itemize}
\item Report the results.
\begin{itemize}
\item Evaluate once on the test split.
\item Plot residuals against predicted values.
\end{itemize}
\end{enumerate}
`,
      String.raw`% Compare two views of a training run
\begin{figure}[htbp]
\centering
\begin{subfigure}{.47\linewidth}
\centering
\includegraphics[width=\linewidth]{training-loss}
\caption{Training loss by epoch.}
\label{fig:loss}
\end{subfigure}
\hfill
\begin{subfigure}{.47\linewidth}
\centering
\includegraphics[width=\linewidth]{validation-accuracy}
\caption{Validation accuracy by epoch.}
\label{fig:accuracy}
\end{subfigure}
\caption{Learning curves for the selected model.}
\label{fig:learning}
\end{figure}

Figure~\ref{fig:learning} shows both curves.
The loss in panel~\ref{fig:loss} stabilizes first.
`,
      String.raw`% Apply a rotation and a scaling
\section{Planar transformations}
A quarter turn followed by a stretch changes a point as follows.
\[
R=\begin{pmatrix}
0&-1\\
1&0
\end{pmatrix},
\qquad
S=\begin{pmatrix}
2&0\\
0&3
\end{pmatrix}.
\]

\begin{align}
p&=\begin{pmatrix}2\\1\end{pmatrix},\\
Rp&=\begin{pmatrix}-1\\2\end{pmatrix},\\
SRp&=\begin{pmatrix}-2\\6\end{pmatrix}.
\end{align}
The order matters because $SR\ne RS$.
`,
    ],
    bibtex: [
      `% Journal articles on information theory
@article{shannon1948,
author={Claude E. Shannon},
title={A Mathematical Theory of Communication},
journal={Bell System Technical Journal},
volume={27},
number={3},
pages={379--423},
year={1948},
doi={10.1002/j.1538-7305.1948.tb01338.x}
}

@article{huffman1952,
author={David A. Huffman},
title={A Method for the Construction of Minimum-Redundancy Codes},
journal={Proceedings of the IRE},
volume={40},
number={9},
pages={1098--1101},
year={1952},
doi={10.1109/JRPROC.1952.273898}
}
`,
      `% Reuse a publisher across book entries
@string{aw="Addison-Wesley"}

@book{knuth1984,
title={The {TeXbook}},
author={Donald E. Knuth},
publisher=aw,
series={Computers and Typesetting},
volume={A},
year={1984}
}

@book{lamport1994,
title={{LaTeX}: A Document Preparation System},
author={Leslie Lamport},
publisher=aw,
edition={2},
year={1994},
isbn={978-0-201-52983-8}
}
`,
      `% Conference papers on neural networks
@inproceedings{vaswani2017,
author={Vaswani, Ashish and others},
title={Attention Is All You Need},
booktitle={Advances in Neural Information Processing Systems},
volume={30},
pages={5998--6008},
year={2017}
}

@inproceedings{he2016,
author={He, Kaiming and Zhang, Xiangyu and
Ren, Shaoqing and Sun, Jian},
title={Deep Residual Learning for Image Recognition},
booktitle={Proceedings of the {IEEE} Conference on
Computer Vision and Pattern Recognition},
pages={770--778},
year={2016},
doi={10.1109/CVPR.2016.90}
}
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
        indentWidth: indentation.value === "tabs" ? 4 : Number(indentation.value),
        useTabs: indentation.value === "tabs",
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
  indentation.addEventListener("change", update);
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
        setStatus(data.diagnostic || "Formatted.", data.diagnostic ? "warning" : "");
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
      result = formatSnippet(data.source, data.bibtex, data.indentWidth, data.useTabs);
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
