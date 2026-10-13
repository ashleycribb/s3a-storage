import React, { useRef, useEffect, useMemo } from "react";
import * as d3 from "d3";
import { MatchCard } from "./MatchCard";
import type { LayoutNode } from "./MatchCard";
import type { TreeNode } from "../lib/bracket-d3";

interface BracketCanvasProps {
  layout: {
    nodes: LayoutNode[];
    links: d3.HierarchyPointLink<TreeNode>[];
    nodeWidth: number;
    nodeHeight: number;
  };
  onDropPlayer: (matchId: string, slot: 1 | 2, playerId: string) => void;
  onClickMatch: (matchId: string) => void;
  theme?: "standard" | "allstar" | "championship";
}

export function BracketCanvas({ layout, onDropPlayer, onClickMatch, theme = "allstar" }: BracketCanvasProps) {
  const svgRef = useRef<SVGSVGElement>(null);
  const zoomGroupRef = useRef<SVGGElement>(null);

  useEffect(() => {
    if (!svgRef.current || !zoomGroupRef.current) return;
    
    const zoom = d3.zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.15, 3])
      .on("zoom", (e) => {
         d3.select(zoomGroupRef.current).attr("transform", e.transform);
      });
      
    const svg = d3.select(svgRef.current);
    svg.call(zoom as unknown as (selection: d3.Selection<SVGSVGElement, unknown, null, undefined>) => void);
  }, []);

  // Compute column centers for round headers
  const roundHeaders = useMemo(() => {
    if (!layout.nodes || layout.nodes.length === 0) return [];

    const roundXMap: { [key: number]: number[] } = {};
    layout.nodes.forEach((node) => {
      const r = node.data.round;
      if (!roundXMap[r]) roundXMap[r] = [];
      roundXMap[r].push(node.x);
    });

    const centers = Object.entries(roundXMap).map(([roundStr, xs]) => {
      const round = parseInt(roundStr);
      const avgX = xs.reduce((sum, x) => sum + x, 0) / xs.length;
      return { round, avgX };
    });

    centers.sort((a, b) => a.round - b.round);
    return centers;
  }, [layout.nodes]);

  // Find minimum Y coordinate to position the headers elegantly above the bracket
  const headerY = useMemo(() => {
    if (!layout.nodes || layout.nodes.length === 0) return -60;
    const minY = Math.min(...layout.nodes.map(n => n.y));
    return Math.max(minY - 70, -80);
  }, [layout.nodes]);

  // Mapping theme labels and dates like Image 1 and Image 2
  const getRoundLabel = (roundIndex: number) => {
    if (theme === "championship") {
      const labels = ["ROUND OF 16", "QUARTERFINAL", "SEMIFINAL", "FINAL"];
      return labels[roundIndex - 1] || `ROUND ${roundIndex}`;
    } else {
      const labels = ["Round 1", "Round 2", "Round 3", "Final Round"];
      return labels[roundIndex - 1] || `Round ${roundIndex}`;
    }
  };

  const getRoundDescription = (roundIndex: number) => {
    if (theme === "championship") {
      return "";
    } else {
      const dates = ["December 21", "December 28", "January 06", "January 18"];
      return dates[roundIndex - 1] || "Live Match";
    }
  };

  const canvasBgClass = 
    theme === "allstar" 
      ? "bg-[#0b0c10]" 
      : theme === "championship" 
      ? "bg-[#050505]" 
      : "bg-[#f8fafc]";

  return (
    <div className={`w-full h-full relative overflow-hidden ${canvasBgClass} transition-colors duration-500`}>
      {/* Top Left Navigation/Breadcrumbs Overlay */}
      <div className="absolute top-4 left-4 z-10 p-3 rounded-lg bg-black/40 backdrop-blur-md border border-white/5 space-y-1">
         <div className="text-xs font-bold text-white/70 uppercase tracking-widest flex items-center gap-1.5 font-mono">
            <span className="w-2 h-2 rounded-full bg-indigo-500 animate-pulse"></span>
            {theme === "allstar" ? "AllStar Canvas Arena" : theme === "championship" ? "Championship Matrix" : "Standard Canvas"}
         </div>
         <div className="text-[10px] text-white/50 font-mono">Scroll middle wheel to zoom • Drag anywhere to pan • Click match card to input result</div>
      </div>
      
      <svg ref={svgRef} className="w-full h-full cursor-grab active:cursor-grabbing">
        <defs>
          {/* Neon Glow filters for premium connectors */}
          <filter id="neon-glow" x="-20%" y="-20%" width="140%" height="140%">
            <feGaussianBlur stdDeviation="3" result="blur" />
            <feComposite in="SourceGraphic" in2="blur" operator="over" />
          </filter>
          <filter id="neon-blue-glow" x="-20%" y="-20%" width="140%" height="140%">
            <feGaussianBlur stdDeviation="4" result="blur" />
            <feComposite in="SourceGraphic" in2="blur" operator="over" />
          </filter>
        </defs>

        <g ref={zoomGroupRef}>
          {/* Column/Round headers positioned at headerY directly above matches */}
          {roundHeaders.map(({ round, avgX }) => (
            <g key={`round-hdr-${round}`} transform={`translate(${avgX}, ${headerY})`}>
              {theme === "allstar" ? (
                <>
                  <text
                    x={layout.nodeWidth / 2}
                    y="0"
                    textAnchor="middle"
                    className="fill-gray-100 font-bold text-sm tracking-wide uppercase font-sans"
                  >
                    {getRoundLabel(round)}
                  </text>
                  <text
                    x={layout.nodeWidth / 2}
                    y="16"
                    textAnchor="middle"
                    className="fill-zinc-500 text-xs font-medium font-mono"
                  >
                    {getRoundDescription(round)}
                  </text>
                  <line
                    x1={0}
                    y1={25}
                    x2={layout.nodeWidth}
                    y2={25}
                    stroke="#27272a"
                    strokeWidth="1"
                  />
                </>
              ) : theme === "championship" ? (
                <>
                  <text
                    x={layout.nodeWidth / 2}
                    y="0"
                    textAnchor="middle"
                    className="fill-stone-400 text-xs tracking-widest font-mono font-bold"
                  >
                    {getRoundLabel(round)}
                  </text>
                  <line
                    x1={layout.nodeWidth / 4}
                    y1={10}
                    x2={(layout.nodeWidth * 3) / 4}
                    y2={10}
                    stroke="#ef4444"
                    strokeWidth="1.5"
                  />
                </>
              ) : (
                <>
                  <text
                    x={layout.nodeWidth / 2}
                    y="0"
                    textAnchor="middle"
                    className="fill-gray-600 font-semibold text-xs tracking-wider uppercase font-sans"
                  >
                    {getRoundLabel(round)}
                  </text>
                  <text
                    x={layout.nodeWidth / 2}
                    y="14"
                    textAnchor="middle"
                    className="fill-gray-400 text-[10px] uppercase tracking-wider"
                  >
                    {getRoundDescription(round)}
                  </text>
                </>
              )}
            </g>
          ))}

          {/* Connectors (Links) */}
          {layout.links.map((link, i) => {
            const source = link.source as LayoutNode; 
            const target = link.target as LayoutNode;
            
            const startX = source.x + layout.nodeWidth;
            const startY = source.y + layout.nodeHeight / 2;
            const targetX = target.x;
            const targetY = target.y + layout.nodeHeight / 2;
            const midX = startX + (targetX - startX) / 2;

            // Smart logic to identify if this is the path of the winner advancing to the parent match
            const sourceWinnerId = source.data.winnerId;
            const isWinnerLink = sourceWinnerId && (
              (target.data.player1 && target.data.player1.id === sourceWinnerId) ||
              (target.data.player2 && target.data.player2.id === sourceWinnerId)
            );

            // Set stroke color and styling based on theme and winner status
            let strokeColor: string;
            let strokeWidth: string;
            let useGlow = false;
            let strokeDash = "";

            if (theme === "allstar") {
              if (isWinnerLink) {
                strokeColor = "#f59e0b"; // Premium Gold
                strokeWidth = "2.5";
                useGlow = true;
              } else {
                strokeColor = "#1e202c"; // Dark carbon gray
                strokeWidth = "1.5";
              }
            } else if (theme === "championship") {
              if (isWinnerLink) {
                // Alternating color curves as shown in Image 2
                strokeColor = i % 2 === 0 ? "#3b82f6" : "#f43f5e"; // Blue / Red vibrant pathways
                strokeWidth = "2.5";
                useGlow = true;
              } else {
                strokeColor = "#1e1e24"; // Charcoal dark
                strokeWidth = "1.5";
                strokeDash = "3,3";
              }
            } else {
              // Standard
              if (isWinnerLink) {
                strokeColor = "#2563eb";
                strokeWidth = "2";
              } else {
                strokeColor = "#e2e8f0";
                strokeWidth = "1.5";
              }
            }

            return (
              <path
                key={`link-${i}`}
                d={`M ${startX} ${startY} C ${midX} ${startY}, ${midX} ${targetY}, ${targetX} ${targetY}`}
                fill="none"
                stroke={strokeColor}
                strokeWidth={strokeWidth}
                strokeDasharray={strokeDash}
                filter={useGlow ? (theme === "allstar" ? "url(#neon-glow)" : "url(#neon-blue-glow)") : undefined}
                className="transition-all duration-300 pointer-events-none"
              />
            );
          })}
          
          {/* Nodes */}
          {layout.nodes.map((node) => {
             const layoutNode = node as LayoutNode;
             return (
               <foreignObject 
                 key={node.data.id}
                 x={layoutNode.x} 
                 y={layoutNode.y} 
                 width={layout.nodeWidth} 
                 height={layout.nodeHeight}
                 className="overflow-visible"
               >
                 <div className="w-full h-full">
                     <MatchCard
                       layoutNode={layoutNode}
                       width={layout.nodeWidth}
                       height={layout.nodeHeight}
                       onDropPlayer={onDropPlayer}
                       onClickMatch={onClickMatch}
                       theme={theme}
                     />
                 </div>
               </foreignObject>
             )
          })}
        </g>
      </svg>
    </div>
  );
}

