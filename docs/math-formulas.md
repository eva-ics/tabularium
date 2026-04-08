# Math Formulas

Tabularium can render LaTeX-style formulas in markdown on the Web UI render surfaces.

Supported forms:

- Inline math with `$...$`
- Block math with `$$...$$`

Inline example:

```md
Water mass-energy relation: $E = mc^2$
```

Block example:

```md
$$
\left( \sum_{k=1}^n a_k b_k \right)^2 \leq
\left( \sum_{k=1}^n a_k^2 \right)
\left( \sum_{k=1}^n b_k^2 \right)
$$
```

Notes:

- Math is rendered in the Web UI preview and chat markdown views.
- Invalid formulas should stay visible as content instead of breaking the page.
- Code fences and inline code keep `$` characters literal.
- GitHub's `$`...``$` inline variant is not supported.
