import fs from 'node:fs';
import path from 'node:path';
import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { LiquidGlassSvgFilters } from '../components/LiquidGlassSvgFilters';

describe('Apple Liquid Glass Design System', () => {
  it('defines exact refractive surface, dark mode variant, backdrop blur, specular rim lighting, and spring physics in liquid-glass.css', () => {
    const cssPath = path.resolve(process.cwd(), 'src/styles/liquid-glass.css');
    expect(fs.existsSync(cssPath)).toBe(true);
    const css = fs.readFileSync(cssPath, 'utf-8');

    expect(css).toContain(
      'linear-gradient(135deg, rgba(255, 255, 255, 0.22) 0%, rgba(255, 255, 255, 0.06) 100%)'
    );
    expect(css).toContain('rgba(28, 30, 38, 0.58)');
    expect(css).toContain('rgba(18, 20, 26, 0.36)');
    expect(css).toContain(
      'blur(28px) saturate(190%) contrast(104%) url(#liquid-glass-refraction)'
    );
    expect(css).toContain(
      'inset 0 1px 0 0 rgba(255, 255, 255, 0.45), inset 0 -1px 0 0 rgba(255, 255, 255, 0.12), 0 20px 45px -15px rgba(0, 0, 0, 0.45)'
    );
    expect(css).toContain('cubic-bezier(0.22, 1.28, 0.36, 1)');
  });

  it('mounts inline SVG filter #liquid-glass-refraction with feTurbulence and feDisplacementMap', () => {
    const { container } = render(<LiquidGlassSvgFilters />);
    const filter = container.querySelector('filter#liquid-glass-refraction');
    expect(filter).not.toBeNull();
    expect(filter?.querySelector('feTurbulence')).not.toBeNull();
    expect(filter?.querySelector('feDisplacementMap')).not.toBeNull();
  });
});
