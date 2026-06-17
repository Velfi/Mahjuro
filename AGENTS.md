# Mahjuro — Agent Notes

## Goals

- Do not break users. Invalidating a run save with an update may be acceptable, but breaking a profile save is not.
- Only change the balance of the game deliberately. If balance changes happen but weren't intentional, they should be reverted.

## Development Principles

- **Small files, happy developers**: Eagerly break down large files (700+ lines) into multiple smaller files while developing the project.
- **No Fallbacks**: things should fail fast and loudly. Don't add fallbacks, they're instant cruft.

## Further Reading

- [Project documentation](docs/)
- [Game design](GAME_DESIGN.md)
- [Architecture](ARCHITECTURE.md)
- [Theme](THEME.md)
