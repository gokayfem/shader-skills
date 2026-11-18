# Testing on a GPU without BC textures

Scene files hold BC7 and BC5 only. A GPU without `texture-compression-bc` gets them unpacked while
loading, at no more than 512 per side. So far that path has only been forced on an RTX 5080, with
`?unpack-textures`. This is what to check on real hardware, such as an Android phone.

**Serve the page over HTTPS.** WebGPU needs a secure context, so a phone can't use a plain `http://`
address on your network. Use a static host, or `adb reverse tcp:8765 tcp:8765` with `localhost`.

**1. Does the device qualify at all?** Run this in the browser's console before anything else:

```js
const a = await navigator.gpu.requestAdapter();
({ bc: a.features.has("texture-compression-bc"),
   etc2: a.features.has("texture-compression-etc2"),
   astc: a.features.has("texture-compression-astc"),
   storageBuffers: a.limits.maxStorageBuffersPerShaderStage,
   storageTextures: a.limits.maxStorageTexturesPerShaderStage,
   maxBuffer: a.limits.maxStorageBufferBindingSize });
```

The renderer refuses to start with fewer than 8 storage buffers or 5 storage textures per stage, which
some phones may have. That would be a renderer limit, not a texture problem, and it would end this test.

**2. Load the small scene first:** `/?scene=web`, which starts small and loads the rest as it goes.

- The Textures line reads `4 x 512, RGBA8 + RG8 unpacked from BC`. If it says `BC7 + BC5`, this GPU
  has BC and the test proves nothing.
- GPU errors stays at 0.
- The image has colour, relief and highlights. Compare with the same view on a desktop: unpacking
  mistakes show as noise or stripes in colour, or as flat, wrongly lit surfaces.

**3. Then the cove:** `/?scene=cove_web`, which starts from a 6 MB download and fetches the rest as
it's needed. Unpacking happens on the page's main thread as textures arrive.

- The Textures line reads `23 x 128 / 256 / 16 x 512, RGBA8 + RG8 unpacked from BC, 65 MB`.
- The tab survives loading. A reload or a "page crashed" message means the phone ran out of memory.
- Note how long it takes between "Preparing the GPU" and the first image, and whether the view
  stutters as textures sharpen.

**4. Report back:** the device and browser, the output of step 1, the Textures line, GPU errors, frame
rate, load time, and a screenshot of the fort wall. Add
`&camera=24.5,1.3,11,1.57,0.02` to the address to match the comparison in the README.
