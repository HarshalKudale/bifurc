const ADJECTIVES = [
  "amber", "blue", "bright", "calm", "clear", "cool", "crisp", "dark",
  "deep", "dry", "fair", "fast", "fine", "fresh", "gold", "green",
  "high", "keen", "kind", "late", "lean", "light", "long", "loud",
  "mild", "neat", "new", "old", "pale", "plain", "pure", "quick",
  "rare", "red", "rich", "sharp", "shy", "slim", "slow", "soft",
  "still", "sweet", "tall", "thin", "warm", "wide", "wild", "wise",
];

const NOUNS = [
  "bay", "brook", "cave", "cliff", "cloud", "coast", "creek", "crest",
  "dale", "dawn", "dew", "dune", "elm", "fern", "field", "flame",
  "flint", "fog", "forge", "frost", "gale", "glen", "grove", "hawk",
  "hill", "lake", "leaf", "marsh", "mist", "moon", "oak", "path",
  "peak", "pine", "pond", "rain", "reed", "ridge", "rock", "sage",
  "sand", "shade", "shore", "sky", "snow", "star", "stone", "storm",
  "sun", "tide", "vale", "wave", "well", "wind", "wood", "wren",
];

export function generateRandomWorkspaceName(): string {
  const adj = ADJECTIVES[Math.floor(Math.random() * ADJECTIVES.length)];
  const noun = NOUNS[Math.floor(Math.random() * NOUNS.length)];
  return `${adj}-${noun}`;
}
