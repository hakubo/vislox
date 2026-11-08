import { Linter } from "eslint";

// Reuse ESLint's parser/scope analysis; no additional parser dependencies.
// Export headers are the sole source of cross-script globals.
export function scriptGlobals(scripts) {
  const owners = new Map(), exports = new Map();
  const linter = new Linter();
  for (const { file, source } of scripts) {
    const messages = linter.verify(source, {
      languageOptions: { ecmaVersion: 2022, sourceType: "script" },
      plugins: { shared: { rules: { collect: { create(context) {
        return { Program() {
          const code = context.sourceCode;
          const bindings = new Set(code.scopeManager.globalScope.variables.filter((v) => v.defs.length).map((v) => v.name));
          for (const name of bindings) {
            if (owners.has(name)) throw new Error(`${file}: duplicate top-level binding ${name} (also in ${owners.get(name)})`);
            owners.set(name, file);
          }
          const names = code.getAllComments()
            .filter((c) => c.type === "Block" && /^\s*exported\b/.test(c.value))
            .flatMap((c) => c.value.replace(/^\s*exported\b/, "").split(",").map((n) => n.trim()).filter(Boolean));
          for (const name of names) {
            if (!bindings.has(name)) throw new Error(`${file}: exported name ${name} has no top-level binding`);
            exports.set(name, file);
          }
        } };
      } } } } },
      rules: { "shared/collect": "error" },
    });
    if (messages.length) throw new Error(`${file}: ${messages[0].message}`);
  }
  return scripts.map(({ file }) => ({
    files: [file],
    languageOptions: { globals: Object.fromEntries([...exports].filter(([, owner]) => owner !== file).map(([name]) => [name, "readonly"])) },
  }));
}
