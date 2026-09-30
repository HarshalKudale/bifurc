/** `bifurc <positionals…> --flag value --flag=value -H value --json`. Flags may repeat. */
export interface ParsedArgs {
  positionals: string[];
  flags: Map<string, string[]>;
}

/** Flags that take no value. */
const BOOLEAN_FLAGS = new Set(["json", "help"]);
/** Single-letter aliases. */
const ALIASES: Record<string, string> = { H: "header", X: "method", d: "body", h: "help", w: "workspace" };

export function parseArgs(argv: readonly string[]): ParsedArgs {
  const positionals: string[] = [];
  const flags = new Map<string, string[]>();
  const add = (name: string, value: string): void => {
    flags.set(name, [...(flags.get(name) ?? []), value]);
  };

  for (let i = 0; i < argv.length; i += 1) {
    const raw = argv[i];
    if (raw === "--") {
      positionals.push(...argv.slice(i + 1));
      break;
    }
    const long = /^--([^=]+)(?:=(.*))?$/s.exec(raw);
    const short = /^-([A-Za-z])$/.exec(raw);
    if (!long && !short) {
      positionals.push(raw);
      continue;
    }
    const name = long ? long[1] : ALIASES[short![1]] ?? short![1];
    if (BOOLEAN_FLAGS.has(name)) {
      add(name, "true");
      continue;
    }
    let value = long?.[2];
    if (value === undefined) {
      value = argv[i + 1];
      if (value === undefined) throw new Error(`--${name} needs a value`);
      i += 1;
    }
    add(name, value);
  }
  return { positionals, flags };
}

export function flag(args: ParsedArgs, name: string): string | undefined {
  const values = args.flags.get(name);
  return values === undefined ? undefined : values[values.length - 1];
}

export function flags(args: ParsedArgs, name: string): string[] {
  return args.flags.get(name) ?? [];
}

export function has(args: ParsedArgs, name: string): boolean {
  return args.flags.has(name);
}
