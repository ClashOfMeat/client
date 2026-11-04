// A verbatim copy of ClashOfMeat/app packages/core/src/heroes/hero-contract.json (docs/hero-contract.md there).
// Change both in the same release. A module rather than JSON, so every system webview can load it.
export const HERO_CONTRACT = Object.freeze({
  "version": 3,
  "frames": { "poses": 16, "walk": 12, "flex": 3, "total": 31, "walkStart": 16, "flexStart": 28, "size": 192 },
  "poses": {
    "0": "stand",
    "1": "kiss lean",
    "2": "sit",
    "3": "unused",
    "4": "talk 1",
    "5": "talk 2",
    "6": "talk 3",
    "7": "kiss approach",
    "8": "kiss (bare: the desktop draws the heart)",
    "9": "grab 1",
    "10": "grab 2",
    "11": "grab 3",
    "12": "angry (the desktop draws the mark)",
    "13": "unused (Flex has its own frames)",
    "14": "sleep",
    "15": "grab held"
  },
  "states": ["idle", "walk", "sit", "talk", "kiss", "angry", "flex", "grab", "sleep"],
  "emotes": ["flex", "kiss", "angry"],
  "clips": {
    "idle": { "frames": [0], "durations": [3000] },
    "walk": {
      "frames": [16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27],
      "durations": [100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100]
    },
    "sit": { "frames": [2], "durations": [3000] },
    "talk": { "frames": [4, 5, 6, 5, 4, 5, 6, 5], "durations": [180, 120, 160, 120, 220, 100, 140, 100] },
    "kiss": { "frames": [0, 1, 7, 8, 7, 1, 0], "durations": [250, 120, 150, 450, 150, 120, 250] },
    "angry": { "frames": [0, 12, 12, 0], "durations": [400, 700, 500, 400] },
    "flex": { "frames": [0, 28, 29, 30], "durations": [300, 125, 125, 3000] },
    "grab": { "frames": [0, 9, 10, 11, 15, 11, 10], "durations": [100, 130, 160, 220, 220, 220, 160] },
    "sleep": { "frames": [2, 14, 14], "durations": [300, 1500, 1600] }
  },
  "visit": { "walkInMs": 1500, "emoteMs": { "kiss": 5000, "angry": 5000, "flex": "one flex clip" }, "walkOutMs": 1500 },
  "limits": { "pngBase64": 65536, "intrinsicPngBase64": 81920, "framesBase64": 1048576, "intrinsicFramesBase64": 1572864 }
})
