// Hero frames are drawn from decoded <img> elements, never ImageBitmaps. WebKit keeps an ImageBitmap's pixels in
// its GPU process; when macOS restarts that process (after sleep, or under memory pressure), every bitmap draws
// nothing while the hero window stays on screen and clickable. An <img> keeps its decoded data in the page, and
// WebKit uploads it again after a restart, so the hero keeps drawing.
export async function decodePng(png, createImage = () => new Image()) {
  const image = createImage()
  image.src = `data:image/png;base64,${png}`
  await image.decode()
  return image
}
