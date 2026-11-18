# Diffuse light exchange in 80 patches

Open the [offline viewer](../studies.html#room-radiosity). Pause, move the lamp from one side of the ceiling to the other, then turn Indirect contribution down to zero. Look at the neutral sphere and the corners where the wall meets the floor. The indirect-radiance view isolates the light that reaches a receiver after reflecting from another surface.

## Running the passes in Shadertoy

1. Paste [Buffer A](buffer-a.glsl) into a Buffer A tab. Bind its iChannel0 to Buffer A for feedback. Use floating-point storage for a custom host.
2. Paste [Image](image.glsl) into the Image tab. Bind its iChannel0 to Buffer A. Both shaders address patch centers using integer texel coordinates. Set PANEL_X consistently in both passes when editing.
3. Restart the simulation. In Shadertoy the buffer continues iterating every frame. The offline host allocates a 20 × 4 buffer, runs the direct pass and three additional passes when lighting changes, then retains the result.

## Reading the buffer

Each group of four columns describes one surface, in order from left wall to right wall, floor, ceiling and back wall. A 4 × 4 grid samples each face. RGB stores outgoing linear diffuse radiance. The sphere receives light and occludes transfer segments. The wall grid does not store radiance emitted by the sphere.

At each patch, integrate the ceiling panel first. For indirect light, gather visible patches from the previous iteration. Both surfaces must face each other. Weight a sender by its area and the two directional cosines, divided by π times squared distance. Cap the total weight at one. This limits near-field energy gain from treating a finite patch as a point. Multiply by the receiving material's diffuse reflectance before writing the next buffer.

For finer contact detail, first inspect the analytic shadow from the direct panel. Raising patch density changes the cost in two places. For P patches, each transport step makes up to P² pair visits, and the image pass makes up to P visits per shaded receiver. The existing solver uses 6,400 candidate pairs per indirect step. Keeping its buffer across frames removes three such sweeps and the direct initialization from each frame with unchanged lighting. The final image still gathers the cached radiance for the visible receiver.
