require("conform").setup({
  formatters_by_ft = {
    tex = { "tift" },
    bib = { "tift" },
  },
  formatters = {
    tift = {
      command = "tift",
      args = { "-", "--stdin-filename", "$FILENAME" },
      stdin = true,
    },
  },
})
