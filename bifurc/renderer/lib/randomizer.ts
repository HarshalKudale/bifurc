/**
 * Randomizer token resolver for template strings.
 * Supports {{random.xxx}} tokens that generate fake/random data at resolve time.
 *
 * Usage in response body / headers:
 *   "Hello {{random.name}}, your ID is {{random.uuid}}"
 */

// -- Token catalogue ---------------------------------------------------------

export interface RandomizerToken {
  key: string;          // e.g. "random.name"
  description: string;  // human-readable label
  example: string;      // sample output shown in UI
}

export const RANDOMIZER_TOKENS: RandomizerToken[] = [
  // Person
  { key: "random.name",        description: "Full name",       example: "Jane Smith" },
  { key: "random.firstName",   description: "First name",      example: "Jane" },
  { key: "random.lastName",    description: "Last name",       example: "Smith" },
  { key: "random.email",       description: "Email address",   example: "jane.smith@example.com" },
  { key: "random.username",    description: "Username",        example: "jane_smith42" },
  { key: "random.phone",       description: "Phone number",    example: "+1-555-867-5309" },
  { key: "random.avatar",      description: "Avatar URL",      example: "https://i.pravatar.cc/150?u=..." },
  // Numbers
  { key: "random.int",         description: "Integer (0–9999)", example: "4217" },
  { key: "random.float",       description: "Decimal (0–1)",   example: "0.8312" },
  { key: "random.boolean",     description: "true / false",    example: "true" },
  { key: "random.price",       description: "Price (USD)",     example: "29.99" },
  // Identifiers
  { key: "random.uuid",        description: "UUID v4",         example: "550e8400-e29b-..." },
  { key: "random.id",          description: "Short ID (8 hex)", example: "a1b2c3d4" },
  // Internet
  { key: "random.url",         description: "URL",             example: "https://example.com/path" },
  { key: "random.ip",          description: "IPv4 address",    example: "192.168.1.42" },
  { key: "random.ipv6",        description: "IPv6 address",    example: "2001:db8::1" },
  { key: "random.color",       description: "Hex color",       example: "#a3f0c2" },
  // Date / time
  { key: "random.date",        description: "ISO date",        example: "2025-08-14" },
  { key: "random.timestamp",   description: "Unix timestamp",  example: "1720000000" },
  { key: "random.isoDateTime", description: "ISO datetime",    example: "2025-08-14T10:30:00Z" },
  // Location
  { key: "random.city",        description: "City name",       example: "Portland" },
  { key: "random.country",     description: "Country name",    example: "Germany" },
  { key: "random.zipCode",     description: "ZIP / postal code", example: "97201" },
  { key: "random.address",     description: "Street address",  example: "742 Evergreen Terrace" },
  // Text
  { key: "random.word",        description: "Random word",     example: "cerulean" },
  { key: "random.sentence",    description: "Short sentence",  example: "The quick brown fox." },
  { key: "random.paragraph",   description: "Paragraph text",  example: "Lorem ipsum..." },
  // Misc
  { key: "random.company",     description: "Company name",    example: "Acme Corp" },
  { key: "random.jobTitle",    description: "Job title",       example: "Senior Engineer" },
  { key: "random.currency",    description: "Currency code",   example: "EUR" },
  { key: "random.locale",      description: "Locale string",   example: "en-US" },
];

// -- Static data pools (no external dep required) ----------------------------

const FIRST_NAMES = ["Alice","Bob","Carol","David","Eve","Frank","Grace","Hank","Iris","Jack","Karen","Leo","Maria","Nate","Olivia","Paul","Quinn","Rachel","Sam","Tina","Uma","Victor","Wendy","Xander","Yara","Zoe"];
const LAST_NAMES  = ["Adams","Baker","Clark","Davis","Evans","Foster","Garcia","Harris","Ingram","Jones","Klein","Lewis","Moore","Nelson","Owen","Parker","Quinn","Reed","Smith","Taylor","Turner","Underwood","Vance","Walker","Young","Zhang"];
const CITIES      = ["New York","Los Angeles","Chicago","Houston","Phoenix","Portland","Seattle","Denver","Austin","Boston","Miami","Atlanta","Detroit","Nashville","Orlando","Minneapolis","Dallas","San Diego","Oakland","Raleigh"];
const COUNTRIES   = ["United States","Germany","France","Canada","Japan","Brazil","Australia","India","United Kingdom","Mexico","Spain","Italy","South Korea","Netherlands","Sweden","Norway","Finland","Denmark","New Zealand","Argentina"];
const COMPANIES   = ["Acme Corp","Globex","Initech","Umbrella Inc","Dunder Mifflin","Pied Piper","Hooli","Vandelay Industries","Soylent Corp","Wayne Enterprises","Stark Industries","Oscorp","LexCorp","Veridian Dynamics"];
const JOB_TITLES  = ["Software Engineer","Product Manager","Designer","Data Scientist","DevOps Engineer","QA Engineer","Solutions Architect","Frontend Developer","Backend Developer","Full Stack Developer","CTO","Engineering Manager"];
const WORDS       = ["cerulean","eloquent","nomadic","fleeting","luminous","serene","resilient","ephemeral","jovial","tenacious","vibrant","wistful","fervent","candid","pensive","effervescent","quixotic","serendipitous"];
const CURRENCIES  = ["USD","EUR","GBP","JPY","CAD","AUD","CHF","CNY","INR","BRL","MXN","KRW","SGD","HKD","NOK","SEK","DKK","PLN","CZK","HUF"];
const LOCALES     = ["en-US","en-GB","de-DE","fr-FR","es-ES","ja-JP","zh-CN","pt-BR","ko-KR","ru-RU","it-IT","nl-NL","sv-SE","pl-PL","ar-SA"];
const SENTENCES   = [
  "The quick brown fox jumps over the lazy dog.",
  "Pack my box with five dozen liquor jugs.",
  "How vexingly quick daft zebras jump!",
  "The five boxing wizards jump quickly.",
  "Sphinx of black quartz, judge my vow.",
];
const PARAGRAPHS  = [
  "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.",
  "Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.",
  "Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur.",
];
const URL_PATHS   = ["/products","/users","/orders","/items","/posts","/articles","/categories","/tags","/search","/settings"];
const URL_DOMAINS = ["example.com","test.io","api.dev","sample.org","demo.net"];
const ZIP_CODES   = ["97201","10001","90210","60601","77001","30301","80201","98101","85001","33101","02101","55401","48201","37201","32801"];

function pick<T>(arr: T[]): T {
  return arr[Math.floor(Math.random() * arr.length)];
}

function randInt(min: number, max: number): number {
  return Math.floor(Math.random() * (max - min + 1)) + min;
}

function hexStr(len: number): string {
  return Array.from({ length: len }, () => Math.floor(Math.random() * 16).toString(16)).join("");
}

// -- Core generator ----------------------------------------------------------

export function generateRandom(key: string): string {
  switch (key) {
    case "random.name":        return `${pick(FIRST_NAMES)} ${pick(LAST_NAMES)}`;
    case "random.firstName":   return pick(FIRST_NAMES);
    case "random.lastName":    return pick(LAST_NAMES);
    case "random.email": {
      const fn = pick(FIRST_NAMES).toLowerCase();
      const ln = pick(LAST_NAMES).toLowerCase();
      const num = randInt(1, 99);
      return `${fn}.${ln}${num}@example.com`;
    }
    case "random.username": {
      const fn = pick(FIRST_NAMES).toLowerCase();
      const num = randInt(1, 999);
      return `${fn}_${num}`;
    }
    case "random.phone": {
      const area = randInt(200, 999);
      const exch = randInt(200, 999);
      const sub  = randInt(1000, 9999);
      return `+1-${area}-${exch}-${sub}`;
    }
    case "random.avatar":
      return `https://i.pravatar.cc/150?u=${hexStr(8)}`;
    case "random.int":
      return String(randInt(0, 9999));
    case "random.float":
      return (Math.random()).toFixed(4);
    case "random.boolean":
      return Math.random() < 0.5 ? "true" : "false";
    case "random.price": {
      const dollars = randInt(1, 999);
      const cents   = randInt(0, 99).toString().padStart(2, "0");
      return `${dollars}.${cents}`;
    }
    case "random.uuid": {
      const p1 = hexStr(8);
      const p2 = hexStr(4);
      const p3 = "4" + hexStr(3);
      const p4 = (8 + Math.floor(Math.random() * 4)).toString(16) + hexStr(3);
      const p5 = hexStr(12);
      return `${p1}-${p2}-${p3}-${p4}-${p5}`;
    }
    case "random.id":
      return hexStr(8);
    case "random.url":
      return `https://${pick(URL_DOMAINS)}${pick(URL_PATHS)}`;
    case "random.ip":
      return `${randInt(1,254)}.${randInt(0,255)}.${randInt(0,255)}.${randInt(1,254)}`;
    case "random.ipv6": {
      const groups = Array.from({ length: 8 }, () => hexStr(4));
      return groups.join(":");
    }
    case "random.color":
      return `#${hexStr(6)}`;
    case "random.date": {
      const d = new Date(Date.now() - randInt(0, 365 * 24 * 60 * 60 * 1000));
      return d.toISOString().slice(0, 10);
    }
    case "random.timestamp":
      return String(Math.floor(Date.now() / 1000) - randInt(0, 86400 * 365));
    case "random.isoDateTime": {
      const d = new Date(Date.now() - randInt(0, 365 * 24 * 60 * 60 * 1000));
      return d.toISOString();
    }
    case "random.city":    return pick(CITIES);
    case "random.country": return pick(COUNTRIES);
    case "random.zipCode": return pick(ZIP_CODES);
    case "random.address":
      return `${randInt(1, 9999)} ${pick(WORDS).charAt(0).toUpperCase() + pick(WORDS).slice(1)} ${pick(["St","Ave","Blvd","Rd","Ln","Dr","Way"])}`;
    case "random.word":      return pick(WORDS);
    case "random.sentence":  return pick(SENTENCES);
    case "random.paragraph": return pick(PARAGRAPHS);
    case "random.company":   return pick(COMPANIES);
    case "random.jobTitle":  return pick(JOB_TITLES);
    case "random.currency":  return pick(CURRENCIES);
    case "random.locale":    return pick(LOCALES);
    default:                 return `{{${key}}}`;
  }
}

// -- Resolver -----------------------------------------------------------------

/**
 * Replaces all {{random.xxx}} tokens in text with generated values.
 * Each occurrence is resolved independently (different random value each time).
 */
export function resolveRandomizers(text: string): string {
  if (!text) return text;
  return text.replace(/\{\{(random\.\w+)\}\}/g, (_match, key) => generateRandom(key));
}
