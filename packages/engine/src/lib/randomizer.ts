/**
 * Randomizer token resolver — Node.js (main process) version.
 * Mirror of renderer/lib/randomizer.ts — kept in sync manually.
 */

// ── Static data pools ────────────────────────────────────────────────────────

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

export function generateRandom(key: string): string {
  switch (key) {
    case "random.name":        return `${pick(FIRST_NAMES)} ${pick(LAST_NAMES)}`;
    case "random.firstName":   return pick(FIRST_NAMES);
    case "random.lastName":    return pick(LAST_NAMES);
    case "random.email": {
      const fn = pick(FIRST_NAMES).toLowerCase();
      const ln = pick(LAST_NAMES).toLowerCase();
      return `${fn}.${ln}${randInt(1, 99)}@example.com`;
    }
    case "random.username": {
      return `${pick(FIRST_NAMES).toLowerCase()}_${randInt(1, 999)}`;
    }
    case "random.phone": {
      return `+1-${randInt(200,999)}-${randInt(200,999)}-${randInt(1000,9999)}`;
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
      return `${randInt(1, 999)}.${randInt(0, 99).toString().padStart(2, "0")}`;
    }
    case "random.uuid": {
      const p1 = hexStr(8);
      const p2 = hexStr(4);
      const p3 = "4" + hexStr(3);
      const p4 = (8 + Math.floor(Math.random() * 4)).toString(16) + hexStr(3);
      const p5 = hexStr(12);
      return `${p1}-${p2}-${p3}-${p4}-${p5}`;
    }
    case "random.id":   return hexStr(8);
    case "random.url":  return `https://${pick(URL_DOMAINS)}${pick(URL_PATHS)}`;
    case "random.ip":   return `${randInt(1,254)}.${randInt(0,255)}.${randInt(0,255)}.${randInt(1,254)}`;
    case "random.ipv6": {
      return Array.from({ length: 8 }, () => hexStr(4)).join(":");
    }
    case "random.color": return `#${hexStr(6)}`;
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

export function resolveRandomizers(text: string): string {
  if (!text) return text;
  return text.replace(/\{\{(random\.\w+)\}\}/g, (_match, key) => generateRandom(key));
}
