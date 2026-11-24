// Three services answer GET /health; the host says which one is meant.
export const check = () => [
  fetch("http://orders:3000/health"),
  fetch("http://payments:3000/health"),
  fetch("http://users:3000/health"),
];
