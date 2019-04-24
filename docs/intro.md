Pathfinder essentially generates a sparse virtual texture of vector are on the fly. The texels are "solid tiles". The reason why we do sparse virtual texturing is that large shapes, which are common in vector art, have a lot of solid and empty areas. It's a waste of memory to allocate explicit space for each tile if most of them are going to be either blank or just a solid color. In fact, one does not even really need to store a tile in memory: one simply generates a shader that would fill a tile with a solid colour.

There are two types of tiles: solid tiles represent coloured tiles, which occlude everything below them. Alpha tiles (also known as "mask tiles", although this is a term being deprecated) sample from solid tiles (the end result being that they apply their mask to a solid tile). Fills are operations that generate alpha masks tiles. Fills are generated on the CPU. Different CPU threads are are given vector objects to process into fills (using Rayon). As soon as a thread finishes generating fills for an object, they are sent off to the GPU (fill operations are commutative, as they are simply addition) which processes fills by affecting the mask framebuffer. 

Possible improvement: generate tiles using the GPU.

Possible improvement: generate fills in reverse z-order (topmost objects to back), so that PF3 can take into account that a tile is occluded, and thus not bother generating fills for it.

The mask framebuffer is essentially a big atlas of alpha masks. The mask framebuffer is a single-channel (i.e. only contains data for alpha) [half-precision float](https://en.wikipedia.org/wiki/Half-precision_floating-point_format) (16 bits per pixel). Note that each tile would have 16^2 = 256 pixels associated with it in the mask framebuffer. Fills add and subtract the values stored within the mask framebuffer, in order to generate representations of "area coverage"

Possible improvement: de-duplicate alpha tiles (i.e. generating a sparse virtual texture of alpha tiles).

Before rendering tiles, Pathfinder throws out all tiles that are covered up by a solid tile above them. This is handled by the ZBuffer struct, which is a tile map that stores the topmost solid tile. The topmost tile is currently determined using the CPU. Note that this occlusion culling occurs at the level of 16x16 tiles: i.e. the CPU side ZBuffer needs to only be 1/256 the size of a GPU side depth buffer (recall that there are 256 pixels per tile, and the GPU-side Z-buffer is per-pixel).


Tiles in Pathfinder have an area of 16 pixels by 16 pixels. Why are tiles 16x16? Having a fixed tile size of 16x16 allows for a very space efficient packing of fills and alpha tile objects. Each fill operation is 64 bits: two sets of (x, y) pairs where x and y are 12 bits each, interpreted in [4.8 fixed point format](http://pfe.sourceforge.net/4thtutor/4thl4-8.htm) (so, 48 bits total for the two (x,y) pairs), and a 16 bits for the tile ID. 
12:57:59 <pcwalton> in other words, 1/256ths of a pixel
12:58:08 <pcwalton> with 16x16 tiles that fits in perfectly :)
