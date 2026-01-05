import { Resvg } from '@resvg/resvg-js';
import { readFileSync, writeFileSync } from 'fs';

const svg = readFileSync('./icon-minimal.svg', 'utf-8');
const resvg = new Resvg(svg, {
  fitTo: {
    mode: 'width',
    value: 256,
  },
});

const pngData = resvg.render();
const pngBuffer = pngData.asPng();

writeFileSync('./icon.png', pngBuffer);
console.log('Created icon.png (256x256)');
