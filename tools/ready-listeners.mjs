// Find registrations executed while a classic script loads. Callback bodies normally
// run later; forEach callbacks run immediately and must be inspected too.
export function loadTimeHandlers(node) {
  const found = [];
  function visit(n) {
    if (!n || typeof n !== "object") return;
    if (["FunctionDeclaration", "FunctionExpression", "ArrowFunctionExpression"].includes(n.type)) return;
    if (n.type === "CallExpression" && n.callee.type === "MemberExpression") {
      if (n.callee.property.name === "addEventListener") {
        found.push({ node: n, handler: n.arguments[1] });
        return;
      }
      if (n.callee.property.name === "forEach") {
        for (const arg of n.arguments) {
          if (["FunctionExpression", "ArrowFunctionExpression"].includes(arg.type)) visit(arg.body);
        }
      }
    }
    if (n.type === "AssignmentExpression" && n.left.type === "MemberExpression" && /^on[a-z]+$/.test(n.left.property.name || "")) {
      found.push({ node: n, handler: n.right });
      return;
    }
    for (const [key, value] of Object.entries(n)) {
      if (key === "parent") continue;
      if (Array.isArray(value)) value.forEach(visit);
      else if (value && typeof value === "object") visit(value);
    }
  }
  visit(node);
  return found;
}

export const readyListeners = {
  create(context) {
    return { Program(node) {
      for (const { node: registration, handler } of loadTimeHandlers(node)) {
        if (handler?.type !== "CallExpression" || handler.callee.name !== "whenReady") {
          context.report({ node: registration, message: "Wrap load-time listeners with whenReady until start.js sets VISLOX_READY." });
        }
      }
    } };
  },
};
