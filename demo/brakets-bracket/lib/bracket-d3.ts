import * as d3 from "d3";

export interface MatchPlayer {
  id?: string;
  name?: string;
  win?: number;
  [key: string]: unknown;
}

export interface RawMatch {
  id: string;
  round: number;
  match: number;
  player1?: MatchPlayer;
  player2?: MatchPlayer;
  active?: boolean;
  path?: {
    win?: string | null;
    [key: string]: unknown;
  };
  [key: string]: unknown;
}

export interface TournamentHelper {
  getPlayer?: (id: string) => { getName?: () => string } | undefined;
}

export interface TreeNode {
  id: string;
  round: number;
  name: string;
  player1: MatchPlayer | undefined;
  player2: MatchPlayer | undefined;
  children: TreeNode[];
  winnerId?: string | null;
  matchData: RawMatch; // original match data
}

export function buildEliminationTree(matches: RawMatch[], tournament?: TournamentHelper): TreeNode[] {
  // Find roots (matches that have no valid path.win in the existing match set)
  // Or simply matches where no other match points to them? No, a root is a match where its path.win is null OR points to a match that doesn't exist (sometimes standard).
  
  const nodes = Object.fromEntries(
    matches.map(m => {
      const p1name = m.player1?.name || (tournament && m.player1?.id ? tournament.getPlayer?.(m.player1.id)?.getName?.() : null);
      const p2name = m.player2?.name || (tournament && m.player2?.id ? tournament.getPlayer?.(m.player2.id)?.getName?.() : null);

      return [
      m.id, 
      { 
        id: m.id, 
        round: m.round, 
        name: `Round ${m.round} - Match ${m.match}`,
        player1: { ...m.player1, name: p1name }, 
        player2: { ...m.player2, name: p2name }, 
        children: [],
        winnerId: m.active ? null : ((m.player1?.win ?? 0) > (m.player2?.win ?? 0) ? m.player1?.id : ((m.player2?.win ?? 0) > (m.player1?.win ?? 0) ? m.player2?.id : null)),
        matchData: m
      } as TreeNode
    ]})
  );

  const roots: TreeNode[] = [];

  // Build links backwards. If match A's win path is match B, then A is a child of B.
  matches.forEach(m => {
    const parentId = m.path?.win;
    const currentNode = nodes[m.id];
    if (currentNode) {
      if (parentId && nodes[parentId]) {
        nodes[parentId].children.push(currentNode);
      } else {
        roots.push(currentNode);
      }
    }
  });

  return roots;
}

export function layoutTree(rootNode: TreeNode, width: number, height: number, nodeWidth: number, nodeHeight: number) {
  const hierarchy = d3.hierarchy<TreeNode>(rootNode);
  const treeLayout = d3.tree<TreeNode>()
    .size([height - nodeHeight * 2, width - nodeWidth * 2])
    .separation((a, b) => (a.parent === b.parent ? 1.5 : 2));

  const pointTree = treeLayout(hierarchy);
  const nodes = pointTree.descendants();
  const links = pointTree.links();

  nodes.forEach(node => {
     const originalX = node.x;
     const originalY = node.y;
     node.x = width - originalY - nodeWidth; 
     node.y = originalX + nodeHeight; 
  });

  return { nodes, links, nodeWidth, nodeHeight };
}
