// Test basic styling using the grid.cell element.

---
// Cell override
#grid(
  align: left,
  fill: red,
  stroke: blue,
  inset: 5pt,
  columns: 2,
  [AAAAA], [BBBBB],
  [A], [B],
  grid.cell(align: right)[C], [D],
  align(right)[E], [F],
  align(horizon)[G], [A\ A\ A],
  grid.cell(align: horizon)[G2], [A\ A\ A],
  grid.cell(inset: 0pt)[I], [F],
  [H], grid.cell(fill: blue)[J]
)

---
// Cell show rule
#show grid.cell: it => [Zz]

#grid(
  align: left,
  fill: red,
  stroke: blue,
  inset: 5pt,
  columns: 2,
  [AAAAA], [BBBBB],
  [A], [B],
  grid.cell(align: right)[C], [D],
  align(right)[E], [F],
  align(horizon)[G], [A\ A\ A]
)

---
#show grid.cell: it => (it.align, it.fill)
#grid(
  align: left,
  row-gutter: 5pt,
  [A],
  grid.cell(align: right)[B],
  grid.cell(fill: aqua)[B],
)

---
// Cell set rules
#set grid.cell(align: center)
#show grid.cell: it => (it.align, it.fill, it.inset)
#set grid.cell(inset: 20pt)
#grid(
  align: left,
  row-gutter: 5pt,
  [A],
  grid.cell(align: right)[B],
  grid.cell(fill: aqua)[B],
)

---
// Test folding per-cell properties (align and inset)
#grid(
  columns: (1fr, 1fr),
  rows: (2.5em, auto),
  align: right,
  inset: 5pt,
  fill: (x, y) => (green, aqua).at(calc.rem(x + y, 2)),
  [Top], grid.cell(align: bottom)[Bot],
  grid.cell(inset: (bottom: 0pt))[Bot], grid.cell(inset: (bottom: 0pt))[Bot]
)

---
// Test overriding outside alignment
#set align(bottom + right)
#grid(
  columns: (1fr, 1fr),
  rows: 2em,
  align: auto,
  fill: green,
  [BR], [BR],
  grid.cell(align: left, fill: aqua)[BL], grid.cell(align: top, fill: red.lighten(50%))[TR]
)

---
// First doc example
#grid(
  columns: 2,
  fill: red,
  align: left,
  inset: 5pt,
  [ABC], [ABC],
  grid.cell(fill: blue)[C], [D],
  grid.cell(align: center)[E], [F],
  [G], grid.cell(inset: 0pt)[H]
)

---
#{
  show grid.cell: emph
  grid(
    columns: 2,
    gutter: 3pt,
    [Hello], [World],
    [Sweet], [Italics]
  )
}
