import React from 'react';

/**
 * Mounts the SVG optical refraction filter referenced by CSS `backdrop-filter: ... url(#liquid-glass-refraction)`.
 * Uses fractal noise + subtle displacement to create liquid-edge caustic distortion on glass surfaces.
 */
export const LiquidGlassSvgFilters: React.FC = () => {
  return (
    <svg
      aria-hidden="true"
      width="0"
      height="0"
      style={{ position: 'absolute', width: 0, height: 0, pointerEvents: 'none', overflow: 'hidden' }}
    >
      <defs>
        <filter
          id="liquid-glass-refraction"
          x="-8%"
          y="-8%"
          width="116%"
          height="116%"
          colorInterpolationFilters="sRGB"
        >
          <feTurbulence
            type="fractalNoise"
            baseFrequency="0.014 0.022"
            numOctaves={2}
            seed={7}
            result="refractionNoise"
          />
          <feGaussianBlur in="refractionNoise" stdDeviation="1.2" result="softNoise" />
          <feDisplacementMap
            in="SourceGraphic"
            in2="softNoise"
            scale={6}
            xChannelSelector="R"
            yChannelSelector="G"
          />
        </filter>
      </defs>
    </svg>
  );
};
