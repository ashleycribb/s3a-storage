// demo/brakets-bracket/packages/core/src/tournament.ts

export function validateMatchScore(score: number[]): boolean {
    // Example implementation of validateMatchScore
    // This is just a placeholder and should be replaced with actual logic
    return score.length === 2 && score.every(s => typeof s === 'number');
}

// You can add more functions or types related to tournaments here