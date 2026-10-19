// demo/brakets-bracket/packages/core/src/tournament.ts

// Example implementation of validateMatchScore
export function validateMatchScore(score: number[]): boolean {
    // Add your validation logic here
    return score.length === 2 && score.every(s => typeof s === 'number');
}

// Other tournament-related code can go here