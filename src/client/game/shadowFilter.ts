import * as THREE from 'three';

/**
 * three.js filters PCF shadows with 5 taps on a Vogel disk rotated per pixel by interleaved
 * gradient noise: a dotted pattern across every shadow that the temporal anti-aliasing cannot
 * average (the noise is fixed in screen space) and that shimmers as the camera jitters. Replaced
 * with a smooth 7×7 tent from 16 bilinear hardware-PCF taps (I. Castaño, "Shadow Mapping Summary",
 * The Witness): no noise, stable under jitter. (The 5×5 one, 9 taps, was too narrow to hide the
 * texel steps of the map: shadow edges looked jagged and crawled on moving beans.)
 */
const TENT = /* glsl */ `// Smooth 7x7 tent from 16 bilinear PCF taps (see shadowFilter.ts).
				vec2 tentUv = shadowCoord.xy * shadowMapSize;
				vec2 tentInv = 1.0 / shadowMapSize;
				vec2 tentBase = floor( tentUv + 0.5 );
				float tentS = tentUv.x + 0.5 - tentBase.x;
				float tentT = tentUv.y + 0.5 - tentBase.y;
				tentBase = ( tentBase - 0.5 ) * tentInv;
				vec4 tentUw = vec4( 5.0 * tentS - 6.0, 11.0 * tentS - 28.0, - ( 11.0 * tentS + 17.0 ), - ( 5.0 * tentS + 1.0 ) );
				vec4 tentU = vec4( ( 4.0 * tentS - 5.0 ) / tentUw.x - 3.0, ( 4.0 * tentS - 16.0 ) / tentUw.y - 1.0, - ( 7.0 * tentS + 5.0 ) / tentUw.z + 1.0, - tentS / tentUw.w + 3.0 ) * tentInv.x;
				vec4 tentVw = vec4( 5.0 * tentT - 6.0, 11.0 * tentT - 28.0, - ( 11.0 * tentT + 17.0 ), - ( 5.0 * tentT + 1.0 ) );
				vec4 tentV = vec4( ( 4.0 * tentT - 5.0 ) / tentVw.x - 3.0, ( 4.0 * tentT - 16.0 ) / tentVw.y - 1.0, - ( 7.0 * tentT + 5.0 ) / tentVw.z + 1.0, - tentT / tentVw.w + 3.0 ) * tentInv.y;
				shadow = 0.0;
				for ( int j = 0; j < 4; j ++ ) {
					for ( int i = 0; i < 4; i ++ ) {
						shadow += tentUw[ i ] * tentVw[ j ] * texture( shadowMap, vec3( tentBase + vec2( tentU[ i ], tentV[ j ] ), shadowCoord.z ) );
					}
				}
				shadow /= 2704.0;`;

const NOISY =
  /float phi = interleavedGradientNoise\( gl_FragCoord\.xy \) \* PI2;\s*shadow = \(\s*texture\( shadowMap, vec3\( shadowCoord\.xy \+ vogelDiskSample[\s\S]*?\) \* 0\.2;/;

let installed = false;

/** Swaps the filter in three's shader chunk (before any material compiles). */
export function installShadowFilter() {
  if (installed) return;
  installed = true;
  const chunk = THREE.ShaderChunk.shadowmap_pars_fragment;
  if (!NOISY.test(chunk)) {
    console.warn('shadowFilter: three.js PCF code not found (three updated?): keeping its filter');
    return;
  }
  (THREE.ShaderChunk as Record<string, string>).shadowmap_pars_fragment = chunk.replace(NOISY, TENT);
}
