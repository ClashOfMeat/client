export const XP_COLOR = 'rgb(207,99,255)'

const XP_GLYPHS={
 'X':['10001','01010','00100','00100','00100','01010','10001'],
 'P':['11110','10001','10001','11110','10000','10000','10000'],
 ':':['00000','00100','00100','00000','00100','00100','00000'],
 '+':['00000','00100','00100','11111','00100','00100','00000'],
 '0':['01110','10001','10011','10101','11001','10001','01110'],
 '1':['00100','01100','00100','00100','00100','00100','01110'],
 '2':['01110','10001','00001','00010','00100','01000','11111'],
 '3':['11110','00001','00001','01110','00001','00001','11110'],
 '4':['00010','00110','01010','10010','11111','00010','00010'],
 '5':['11111','10000','10000','11110','00001','00001','11110'],
 '6':['01110','10000','10000','11110','10001','10001','01110'],
 '7':['11111','00001','00010','00100','01000','01000','01000'],
 '8':['01110','10001','10001','01110','10001','10001','01110'],
 '9':['01110','10001','10001','01111','00001','00001','01110'],
}

export function xpTextFrame(amount, elapsed, { centerX, areaHeight, scale = 2, maxWidth = Infinity }) {
  if (!Number.isSafeInteger(amount) || amount <= 0 || elapsed < 0 || elapsed >= 1800) return null
  const text = `XP: +${amount}`
  const width = (text.length * 6 - 1) * scale, height = 7 * scale
  const left = Math.max(0, Math.min(Math.round(centerX - text.length * 6 * scale / 2), Math.max(0, maxWidth - width)))
  const top = Math.round(Math.max(0, areaHeight - height) * (1 - Math.min(1, elapsed / 1800)))
  const alpha = Math.min(1, Math.max(0, (1800 - elapsed) / 350))
  const pixels = []
  for (let i = 0; i < text.length; i++) {
    const glyph = XP_GLYPHS[text[i]]
    if (!glyph) continue
    for (let y = 0; y < 7; y++) for (let x = 0; x < 5; x++) {
      if (glyph[y][x] === '1') pixels.push([i * 6 + x, y])
    }
  }
  return { text, alpha, left, top, width, height, scale, pixels }
}

export function drawXpText(ctx, frame) {
  if (frame === null) return
  ctx.save()
  ctx.globalAlpha = frame.alpha
  ctx.fillStyle = XP_COLOR
  for (const [x, y] of frame.pixels) ctx.fillRect(frame.left + x * frame.scale, frame.top + y * frame.scale, frame.scale, frame.scale)
  ctx.restore()
}
