# Satin & light

Same geometry as today: capture brackets, one text line, one accent.

- **Gradient**: a graphite tile lit from above, with a faint upper-left sheen. The frame shares one satin falloff, bright upper-left to cool grey lower-right.
- **Texture**: about 4 % feTurbulence grain on the tile.
- **Material**: satin brackets and line, from a shallow lighting filter that rounds only their edges (no inflated plastic). The tile has a lit rim.
- **Light**: the accent is a short rod, ink blue → cyan → peach tip. It glows onto the tile and tints the line's lower left.
- **Small sizes**: flat colours, with the gradient only on the accent. The falloff becomes three bracket tones; 32/24 get a hairline edge for dark taskbars.
- **Risks**: grain aliases below about 64 px, so downsample from 1024. At 16 px the gradient is two shades, and the line is dropped.
