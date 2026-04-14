import type { NodeEditor, GetSchemes, ClassicPreset } from "rete";
import { HardwareNode, COMPONENT_TYPES } from "./nodes-hardware";
import { ConnectorNode } from "./nodes-connector";
import {
  VarNode, NumberNode, MathNode, CompareNode,
  LogicNode, NotNode, MaskBuiltinNode, RangeNode,
} from "./nodes-math";
import type { TextControl, NumberControl } from "./controls";

// ─── Types ────────────────────────────────────────────────────────────────────

interface ConfigEntry {
  key: string;
  value: string;
}

interface ComponentDef {
  name: string;
  type: string;
  config?: ConfigEntry[];
  children?: ComponentDef[];
}

interface ForRange {
  var_name: string;
  start: number;
  end: number;
}

interface LinkDef {
  pattern?: string;
  src?: string;
  dst?: string;
  for_ranges?: ForRange[];
  where_expr?: string;
  latency: number;
  weight: number;
}

interface TopologyDef {
  root: ComponentDef;
  links: LinkDef[];
}

interface SimulationConfig {
  max_ticks: number;
  num_threads: number;
  exec_mode: string;
  vm: { arch: string };
  topology: TopologyDef;
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

type AnyNode = HardwareNode | ConnectorNode | VarNode | NumberNode | MathNode
  | CompareNode | LogicNode | NotNode | MaskBuiltinNode | RangeNode;
type Conn = ClassicPreset.Connection<AnyNode, AnyNode>;
type Schemes = GetSchemes<AnyNode, Conn>;

function getCtrlText(node: AnyNode, key: string): string {
  const ctrl = node.controls[key] as TextControl | undefined;
  return ctrl?.value ?? "";
}

function getCtrlNumber(node: AnyNode, key: string): number {
  const ctrl = node.controls[key] as NumberControl | undefined;
  return ctrl?.value ?? 0;
}

/** Walk upstream connections to find which HardwareNode + socket key feeds an input. */
function traceHwSocket(
  editor: NodeEditor<Schemes>,
  nodeId: string,
  inputKey: string,
): { node: HardwareNode; socketKey: string } | null {
  const conn = editor
    .getConnections()
    .find((c) => c.target === nodeId && c.targetInput === inputKey);
  if (!conn) return null;
  const srcNode = editor.getNode(conn.source);
  if (srcNode instanceof HardwareNode) {
    return { node: srcNode, socketKey: conn.sourceOutput };
  }
  return null;
}

/** Build a hierarchical name path for a HardwareNode by walking parent connections. */
function buildPath(
  editor: NodeEditor<Schemes>,
  nodeId: string,
  cache: Map<string, string>,
): string {
  if (cache.has(nodeId)) return cache.get(nodeId)!;
  const node = editor.getNode(nodeId);
  if (!(node instanceof HardwareNode)) return "";

  const name = node.instanceName;
  // Find parent connection (this node's "parent" output → some node's "children" input)
  const parentConn = editor
    .getConnections()
    .find((c) => c.source === nodeId && c.sourceOutput === "parent");
  if (!parentConn) {
    cache.set(nodeId, name);
    return name;
  }
  const parentPath = buildPath(editor, parentConn.target, cache);
  const fullPath = parentPath ? `${parentPath}.${name}` : name;
  cache.set(nodeId, fullPath);
  return fullPath;
}

/** Evaluate an expression sub-graph into a string expression. */
function evalExpr(editor: NodeEditor<Schemes>, nodeId: string): string {
  const node = editor.getNode(nodeId);

  if (node instanceof VarNode) {
    return getCtrlText(node, "varName") || "i";
  }
  if (node instanceof NumberNode) {
    return String(getCtrlNumber(node, "value"));
  }
  if (node instanceof RangeNode) {
    return getCtrlText(node, "varName") || "i";
  }

  // Binary expressions
  if (node instanceof MathNode) {
    const opMap: Record<string, string> = {
      add: "+", sub: "-", mul: "*", div: "/", mod: "%",
    };
    const a = traceExprInput(editor, node.id, "a");
    const b = traceExprInput(editor, node.id, "b");
    return `${a}${opMap[node.op]}${b}`;
  }
  if (node instanceof CompareNode) {
    const opMap: Record<string, string> = {
      eq: "==", ne: "!=", gt: ">", lt: "<", gte: ">=", lte: "<=",
    };
    const a = traceExprInput(editor, node.id, "a");
    const b = traceExprInput(editor, node.id, "b");
    return `${a} ${opMap[node.op]} ${b}`;
  }
  if (node instanceof LogicNode) {
    const a = traceExprInput(editor, node.id, "a");
    const b = traceExprInput(editor, node.id, "b");
    const op = node.op === "and" ? "&&" : "||";
    return `${a} ${op} ${b}`;
  }
  if (node instanceof NotNode) {
    const a = traceExprInput(editor, node.id, "a");
    return `!(${a})`;
  }
  if (node instanceof MaskBuiltinNode) {
    // These don't directly produce a where_expr — they inform the mask strategy.
    // Return a placeholder; the connector serializer handles the details.
    return `__mask_${node.builtin}__`;
  }

  return "?";
}

function traceExprInput(
  editor: NodeEditor<Schemes>,
  nodeId: string,
  inputKey: string,
): string {
  const conn = editor
    .getConnections()
    .find((c) => c.target === nodeId && c.targetInput === inputKey);
  if (!conn) return "0";
  return evalExpr(editor, conn.source);
}

/** Collect all RangeNodes reachable from a connector's mask sub-graph. */
function collectRanges(
  editor: NodeEditor<Schemes>,
  nodeId: string,
  visited: Set<string>,
): ForRange[] {
  if (visited.has(nodeId)) return [];
  visited.add(nodeId);

  const node = editor.getNode(nodeId);
  if (node instanceof RangeNode) {
    return [{
      var_name: getCtrlText(node, "varName") || "i",
      start: getCtrlNumber(node, "start"),
      end: getCtrlNumber(node, "end"),
    }];
  }

  // Recurse through all incoming expression connections
  const ranges: ForRange[] = [];
  const incoming = editor
    .getConnections()
    .filter((c) => c.target === nodeId);
  for (const conn of incoming) {
    ranges.push(...collectRanges(editor, conn.source, visited));
  }
  return ranges;
}

/** Deduplicate ranges by var_name. */
function dedupeRanges(ranges: ForRange[]): ForRange[] {
  const seen = new Map<string, ForRange>();
  for (const r of ranges) {
    seen.set(r.var_name, r);
  }
  return [...seen.values()];
}

// ─── Build the socket reference string ────────────────────────────────────────

function buildSocketRef(
  editor: NodeEditor<Schemes>,
  connectorId: string,
  inputKey: "from" | "to",
  pathCache: Map<string, string>,
): string {
  const ref = traceHwSocket(editor, connectorId, inputKey);
  if (!ref) return "???";
  const path = buildPath(editor, ref.node.id, pathCache);
  // Extract hardware socket name from the key: "sock_req_[n]" → "req_[n]"
  const sockName = ref.socketKey.replace(/^sock_/, "");
  return `${path}.${sockName}`;
}

// ─── Serialize mask / where_expr ──────────────────────────────────────────────

function serializeMask(
  editor: NodeEditor<Schemes>,
  connectorId: string,
): { where_expr?: string; for_ranges: ForRange[] } {
  const maskConn = editor
    .getConnections()
    .find((c) => c.target === connectorId && c.targetInput === "mask");

  if (!maskConn) return { for_ranges: [] };

  const maskNodeId = maskConn.source;
  const maskNode = editor.getNode(maskNodeId);

  // Collect all ranges from the expression sub-graph
  const ranges = dedupeRanges(collectRanges(editor, maskNodeId, new Set()));

  // If the mask is a MaskBuiltinNode, we handle specially:
  if (maskNode instanceof MaskBuiltinNode) {
    if (maskNode.builtin === "cross_no_self") {
      // Need a where_expr like "i != j"
      const varA = traceExprInput(editor, maskNodeId, "varA");
      const varB = traceExprInput(editor, maskNodeId, "varB");
      return {
        for_ranges: ranges,
        where_expr: `${varA} != ${varB}`,
      };
    }
    return { for_ranges: ranges };
  }

  // If the top-level mask is a comparison, it becomes the where_expr
  if (maskNode instanceof CompareNode || maskNode instanceof LogicNode || maskNode instanceof NotNode) {
    const expr = evalExpr(editor, maskNodeId);
    return {
      for_ranges: ranges,
      where_expr: expr,
    };
  }

  return { for_ranges: ranges };
}

// ─── Main export function ─────────────────────────────────────────────────────

export function exportConfig(editor: NodeEditor<Schemes>): SimulationConfig {
  const pathCache = new Map<string, string>();

  // ── Find the root SOC node ──────────────────────────────────────────────

  const allNodes = editor.getNodes();
  const socNode = allNodes.find(
    (n) => n instanceof HardwareNode && n.componentType === "soc",
  ) as HardwareNode | undefined;

  if (!socNode) {
    throw new Error("No SOC node found — graph needs a root SOC component.");
  }

  // ── Build component tree recursively ────────────────────────────────────

  function buildComponent(node: HardwareNode): ComponentDef {
    const count = getCtrlNumber(node, "count");
    const name =
      count > 1
        ? `${node.instanceName}[0:${count}]`
        : node.instanceName;

    const typeDef = COMPONENT_TYPES.find((t) => t.type === node.componentType);

    const config: ConfigEntry[] = [];
    if (typeDef) {
      for (const c of typeDef.configs) {
        const val = getCtrlText(node, `cfg_${c.key}`);
        if (val && val !== c.defaultValue) {
          config.push({ key: c.key, value: val });
        } else if (val) {
          config.push({ key: c.key, value: val });
        }
      }
    }

    // Find children: nodes whose "parent" output connects to this node's "children" input
    const childConns = editor
      .getConnections()
      .filter((c) => c.target === node.id && c.targetInput === "children");

    const children: ComponentDef[] = [];
    for (const cc of childConns) {
      const childNode = editor.getNode(cc.source);
      if (childNode instanceof HardwareNode) {
        children.push(buildComponent(childNode));
      }
    }

    const def: ComponentDef = { name, type: node.componentType };
    if (config.length > 0) def.config = config;
    if (children.length > 0) def.children = children;
    return def;
  }

  const root = buildComponent(socNode);

  // ── Build links from ConnectorNodes ─────────────────────────────────────

  const links: LinkDef[] = [];
  const connectorNodes = allNodes.filter(
    (n) => n instanceof ConnectorNode,
  ) as ConnectorNode[];

  for (const conn of connectorNodes) {
    const fromRef = buildSocketRef(editor, conn.id, "from", pathCache);
    const toRef = buildSocketRef(editor, conn.id, "to", pathCache);
    const latency = getCtrlNumber(conn, "latency");
    const weight = getCtrlNumber(conn, "weight");

    const { for_ranges, where_expr } = serializeMask(editor, conn.id);

    const pattern = `${fromRef} -> ${toRef}`;
    const link: LinkDef = { pattern, latency, weight };
    if (for_ranges.length > 0) link.for_ranges = for_ranges;
    if (where_expr) link.where_expr = where_expr;
    links.push(link);
  }

  // ── Assemble top-level config ───────────────────────────────────────────

  const arch = getCtrlText(socNode, "cfg_arch") || "cdna4";

  return {
    max_ticks: 100000,
    num_threads: 1,
    exec_mode: "functional",
    vm: { arch },
    topology: { root, links },
  };
}
