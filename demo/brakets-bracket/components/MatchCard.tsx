import React from "react";
import type { TreeNode } from "../lib/bracket-d3";
import type * as d3 from "d3";

export type LayoutNode = d3.HierarchyPointNode<TreeNode>;

interface MatchCardProps {
  layoutNode: LayoutNode;
  width?: number;
  height?: number;
  onDropPlayer: (matchId: string, slot: 1 | 2, playerId: string) => void;
  onClickMatch: (matchId: string) => void;
  theme?: "standard" | "allstar" | "championship";
}

// Preset mapping for esports teams
const TEAM_PRESETS: { [key: string]: { logo: string; gradient: string; tint: string } } = {
  dignitas: { logo: "🐱", gradient: "from-yellow-400 to-amber-500", tint: "bg-yellow-500/20 text-yellow-400" },
  spotfire: { logo: "⚡", gradient: "from-sky-400 to-cyan-500", tint: "bg-cyan-500/20 text-cyan-300" },
  phoenix: { logo: "🔥", gradient: "from-orange-500 to-red-600", tint: "bg-orange-500/20 text-orange-400" },
  tricked: { logo: "👹", gradient: "from-purple-500 to-indigo-600", tint: "bg-purple-500/20 text-purple-300" },
  valiant: { logo: "👑", gradient: "from-yellow-300 to-yellow-500", tint: "bg-yellow-400/20 text-yellow-300" },
  montreal: { logo: "🛡️", gradient: "from-rose-500 to-red-700", tint: "bg-red-500/20 text-red-300" },
  canids: { logo: "🐺", gradient: "from-red-400 to-rose-600", tint: "bg-red-500/20 text-red-400" },
  enigma: { logo: "🌀", gradient: "from-violet-500 to-indigo-700", tint: "bg-indigo-500/20 text-indigo-300" },
};

function getInitials(name?: string): string {
  if (!name) return "?";
  const parts = name.trim().split(/\s+/);
  return parts.length >= 2 ? (parts[0][0] + parts[1][0]).toUpperCase() : name.slice(0, 2).toUpperCase();
}

export function MatchCard({ layoutNode, onDropPlayer, onClickMatch, theme = "allstar" }: MatchCardProps) {
  const match = layoutNode.data;
  
  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault();
  };

  const handleDrop = (e: React.DragEvent, slot: 1 | 2) => {
    e.preventDefault();
    const playerId = e.dataTransfer.getData("playerId");
    if (playerId) {
      onDropPlayer(match.id, slot, playerId);
    }
  };

  const isRound1 = match.children && match.children.length === 0;

  // Helper to extract styled team assets
  const getTeamPreset = (name: string | undefined) => {
    if (!name) return null;
    const clean = name.toLowerCase().trim();
    return TEAM_PRESETS[clean] || { logo: "🎮", gradient: "from-gray-500 to-zinc-600", tint: "bg-gray-500/20 text-gray-400" };
  };

  // ------------------------------------------------------------------------
  // RENDER THEME A: "ALLSTAR ARENA"
  // ------------------------------------------------------------------------
  if (theme === "allstar") {
    const p1Preset = getTeamPreset(match.player1?.name);
    const p2Preset = getTeamPreset(match.player2?.name);
    
    const isP1Winner = match.winnerId && match.winnerId === match.player1?.id;
    const isP2Winner = match.winnerId && match.winnerId === match.player2?.id;

    const hasMatchesEnded = (match.player1?.win ?? 0) > 0 || (match.player2?.win ?? 0) > 0;

    return (
      <div
        className="h-full w-full rounded-xl bg-[#141622]/90 border border-zinc-800/80 shadow-lg overflow-hidden flex flex-col justify-between hover:border-[#f59e0b]/50 hover:shadow-[#f59e0b]/5 select-none transition-all duration-300 group cursor-pointer"
        onClick={() => onClickMatch(match.id)}
      >
        {/* Card Header showing match index / state */}
        <div className="bg-[#0b0c11] px-2.5 py-1.5 border-b border-zinc-800/60 flex justify-between items-center text-[10px] uppercase font-bold tracking-wider text-zinc-500">
          <span>{match.name.replace("Round ", "R")}</span>
          {!match.matchData?.active ? (
            <span className="text-[#f59e0b] bg-[#f59e0b]/10 px-1 py-0.5 rounded text-[8px] tracking-widest font-mono">FINISHED</span>
          ) : (
            <span className="text-zinc-500 flex items-center gap-1"><span className="w-1.5 h-1.5 rounded-full bg-emerald-500"></span>LIVE</span>
          )}
        </div>

        {/* Both team rows layout centered */}
        <div className="flex-1 flex flex-col justify-center py-1">
          {/* Team 1 strip */}
          <div className={`px-2.5 py-1.5 flex items-center justify-between ${isP1Winner ? 'bg-[#f59e0b]/5' : ''}`}>
            <div className="flex items-center gap-2 max-w-[130px] truncate">
              {match.player1?.name ? (
                <>
                  <div className={`w-5.5 h-5.5 rounded bg-gradient-to-br ${p1Preset?.gradient} flex items-center justify-center text-xs shadow-sm`}>
                    {p1Preset?.logo}
                  </div>
                  <span className={`text-[11px] font-sans font-semibold tracking-wide truncate ${isP1Winner ? 'text-white' : 'text-zinc-400'}`}>
                    {match.player1?.name}
                  </span>
                </>
              ) : (
                <>
                  <div className="w-5.5 h-5.5 rounded bg-[#1e2030] flex items-center justify-center text-xs text-zinc-600 border border-zinc-800 border-dashed">
                    ?
                  </div>
                  <span className="text-[11px] italic text-zinc-600">Waiting team</span>
                </>
              )}
            </div>
            {hasMatchesEnded && match.player1?.id && (
              <span className={`font-mono text-xs font-bold leading-none px-1.5 py-1 rounded text-right ${isP1Winner ? 'text-[#f59e0b] bg-[#f59e0b]/10 font-black' : 'text-zinc-500 bg-zinc-900/40'}`}>
                {match.player1.win}
              </span>
            )}
          </div>

          {/* Team 2 strip */}
          <div className={`px-2.5 py-1.5 flex items-center justify-between ${isP2Winner ? 'bg-[#f59e0b]/5' : ''}`}>
            <div className="flex items-center gap-2 max-w-[130px] truncate">
              {match.player2?.name ? (
                <>
                  <div className={`w-5.5 h-5.5 rounded bg-gradient-to-br ${p2Preset?.gradient} flex items-center justify-center text-xs shadow-sm`}>
                    {p2Preset?.logo}
                  </div>
                  <span className={`text-[11px] font-sans font-semibold tracking-wide truncate ${isP2Winner ? 'text-white' : 'text-zinc-400'}`}>
                    {match.player2?.name}
                  </span>
                </>
              ) : (
                <>
                  <div className="w-5.5 h-5.5 rounded bg-[#1e2030] flex items-center justify-center text-xs text-zinc-600 border border-zinc-800 border-dashed">
                    ?
                  </div>
                  <span className="text-[11px] italic text-zinc-600">Waiting team</span>
                </>
              )}
            </div>
            {hasMatchesEnded && match.player2?.id && (
              <span className={`font-mono text-xs font-bold leading-none px-1.5 py-1 rounded text-right ${isP2Winner ? 'text-[#f59e0b] bg-[#f59e0b]/10 font-black' : 'text-zinc-500 bg-zinc-900/40'}`}>
                {match.player2.win}
              </span>
            )}
          </div>
        </div>
      </div>
    );
  }

  // ------------------------------------------------------------------------
  // RENDER THEME B: "CHAMPIONSHIP GRID IP"
  // ------------------------------------------------------------------------
  if (theme === "championship") {
    const isP1Winner = match.winnerId && match.winnerId === match.player1?.id;
    const isP2Winner = match.winnerId && match.winnerId === match.player2?.id;

    const hasMatchesEnded = (match.player1?.win ?? 0) > 0 || (match.player2?.win ?? 0) > 0;
    
    return (
      <div 
        className="w-full h-full bg-[#121215] border border-stone-800/80 hover:border-stone-600 rounded-lg overflow-hidden flex flex-col justify-center py-1 relative shadow-2xl group transition-all duration-300 cursor-pointer text-xs"
        onClick={() => onClickMatch(match.id)}
      >
        {/* Player 1 Row */}
        <div 
          className={`px-3 py-1 flex items-center justify-between relative ${isP1Winner ? 'bg-indigo-950/25' : ''}`}
          onDragOver={isRound1 ? handleDragOver : undefined}
          onDrop={isRound1 ? (e) => handleDrop(e, 1) : undefined}
        >
          <div className="flex items-center gap-2.5 max-w-[130px] truncate">
            {match.player1?.name ? (
              <>
                <div className="w-5.5 h-5.5 rounded-full bg-stone-800 text-stone-300 flex items-center justify-center text-[9px] font-bold border border-stone-700/80 flex-shrink-0">
                  {getInitials(match.player1.name)}
                </div>
                <span className={`font-mono tracking-wider font-semibold uppercase truncate text-[11px] ${isP1Winner ? 'text-white font-bold' : 'text-stone-400'}`}>
                  {match.player1.name}
                </span>
              </>
            ) : (
              <>
                <div className="w-5.5 h-5.5 rounded-full bg-stone-900 border border-stone-800 border-dashed flex items-center justify-center text-[8px] text-stone-600">?</div>
                <span className="font-mono text-[10px] uppercase text-stone-600 italic">Waiting...</span>
              </>
            )}
          </div>
          {match.player1?.id && (
            <span className={`font-mono text-center w-6 py-0.5 text-[11px] font-bold rounded ${
              isP1Winner 
                ? 'bg-[#2563eb] text-white shadow-[#2563eb]/20 shadow-md font-extrabold' 
                : isP2Winner 
                ? 'bg-stone-900 text-stone-600 font-semibold'
                : hasMatchesEnded 
                ? 'bg-stone-800 text-stone-300' 
                : 'bg-stone-900/50 text-stone-500'
            }`}>
              {hasMatchesEnded ? match.player1.win : "-"}
            </span>
          )}
        </div>

        {/* Tiny clean divider line inside pill */}
        <div className="mx-2 border-t border-stone-900/60 my-0.5"></div>

        {/* Player 2 Row */}
        <div 
          className={`px-3 py-1 flex items-center justify-between relative ${isP2Winner ? 'bg-indigo-950/25' : ''}`}
          onDragOver={isRound1 ? handleDragOver : undefined}
          onDrop={isRound1 ? (e) => handleDrop(e, 2) : undefined}
        >
          <div className="flex items-center gap-2.5 max-w-[130px] truncate">
            {match.player2?.name ? (
              <>
                <div className="w-5.5 h-5.5 rounded-full bg-stone-800 text-stone-300 flex items-center justify-center text-[9px] font-bold border border-stone-700/80 flex-shrink-0">
                  {getInitials(match.player2.name)}
                </div>
                <span className={`font-mono tracking-wider font-semibold uppercase truncate text-[11px] ${isP2Winner ? 'text-white font-bold' : 'text-stone-400'}`}>
                  {match.player2.name}
                </span>
              </>
            ) : (
              <>
                <div className="w-5.5 h-5.5 rounded-full bg-stone-900 border border-stone-800 border-dashed flex items-center justify-center text-[8px] text-stone-600">?</div>
                <span className="font-mono text-[10px] uppercase text-stone-600 italic">Waiting...</span>
              </>
            )}
          </div>
          {match.player2?.id && (
            <span className={`font-mono text-center w-6 py-0.5 text-[11px] font-bold rounded ${
              isP2Winner 
                ? 'bg-[#2563eb] text-white shadow-[#2563eb]/20 shadow-md font-extrabold' 
                : isP1Winner 
                ? 'bg-stone-900 text-stone-600 font-semibold'
                : hasMatchesEnded 
                ? 'bg-stone-800 text-stone-300' 
                : 'bg-stone-900/50 text-stone-500'
            }`}>
              {hasMatchesEnded ? match.player2.win : "-"}
            </span>
          )}
        </div>
      </div>
    );
  }

  // ------------------------------------------------------------------------
  // RENDER STANDARD THEME (FALLBACK)
  // ------------------------------------------------------------------------
  return (
    <div
      className="bg-white rounded-lg shadow-sm border border-gray-200 overflow-hidden text-sm cursor-pointer hover:shadow-md hover:border-gray-300 transition-all select-none flex flex-col h-full w-full relative"
      onClick={() => onClickMatch(match.id)}
    >
      <div className="bg-gray-100 text-gray-500 text-xs px-2 py-1 font-medium border-b border-gray-200 flex justify-between">
        <span>{match.name}</span>
        {!match.matchData?.active && <span className="text-green-500">✓</span>}
      </div>
      
      <div className="flex-1 flex flex-col">
        {/* Player 1 Slot */}
        <div 
          className={`flex-1 px-3 flex items-center justify-between border-b border-gray-100 ${
            match.winnerId && match.winnerId === match.player1?.id ? "font-bold text-gray-900 bg-green-50/50" : "text-gray-700"
          } ${isRound1 && (!match.player1 || !match.player1.id) ? "bg-gray-50/50 border-dashed border-2 border-gray-200" : ""}`}
          onDragOver={isRound1 ? handleDragOver : undefined}
          onDrop={isRound1 ? (e) => handleDrop(e, 1) : undefined}
        >
          <span className="truncate w-full block">{match.player1?.name ? match.player1.name : "TBD"}</span>
          <span className="font-mono ml-2 text-gray-500">{((match.player1?.win ?? 0) > 0 || (match.player2?.win ?? 0) > 0) ? (match.player1?.win ?? 0) : "-"}</span>
        </div>
        
        {/* Player 2 Slot */}
        <div 
          className={`flex-1 px-3 flex items-center justify-between ${
            match.winnerId && match.winnerId === match.player2?.id ? "font-bold text-gray-900 bg-green-50/50" : "text-gray-700"
          } ${isRound1 && (!match.player2 || !match.player2.id) ? "bg-gray-50/50 border-dashed border-2 border-gray-200" : ""}`}
          onDragOver={isRound1 ? handleDragOver : undefined}
          onDrop={isRound1 ? (e) => handleDrop(e, 2) : undefined}
        >
          <span className="truncate w-full block">{match.player2?.name ? match.player2.name : "TBD"}</span>
          <span className="font-mono ml-2 text-gray-500">{((match.player1?.win ?? 0) > 0 || (match.player2?.win ?? 0) > 0) ? (match.player1?.win ?? 0) : "-"}</span>
        </div>
      </div>
    </div>
  );
}
