/**
 * WebGL2 GPU Cushion & Liquid Glass Specular Shader Renderer.
 * Uploads the van Wijk RGBA cushion buffer to a GPU texture only when layout changes,
 * then composites cursor-linked optical specular sheen and hovered-rectangle refractive
 * rim lighting via lightweight uniform updates (<0.2ms per mouse frame).
 */

export interface WebGL2CushionRenderer {
  uploadTexture(buffer: Uint8ClampedArray, width: number, height: number): void;
  draw(
    width: number,
    height: number,
    cursorX: number,
    cursorY: number,
    hoverRect: [number, number, number, number] | null
  ): void;
  dispose(): void;
}

const VERT_SRC = `#version 300 es
in vec2 a_pos;
out vec2 v_uv;
void main() {
  v_uv = vec2(a_pos.x * 0.5 + 0.5, 1.0 - (a_pos.y * 0.5 + 0.5));
  gl_Position = vec4(a_pos, 0.0, 1.0);
}
`;

const FRAG_SRC = `#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_tex;
uniform vec2 u_resolution;
uniform vec2 u_cursor;
uniform vec4 u_hoverRect;
uniform float u_hasHover;
out vec4 outColor;

void main() {
  vec4 base = texture(u_tex, v_uv);
  vec2 px = v_uv * u_resolution;

  // Cursor-linked translucent specular glass spotlight
  float dist = distance(px, u_cursor);
  float spotlight = exp(-dist * 0.0085) * 0.16;
  vec3 color = base.rgb + vec3(0.82, 0.91, 1.0) * spotlight;

  if (u_hasHover > 0.5) {
    vec2 r0 = u_hoverRect.xy;
    vec2 r1 = u_hoverRect.xy + u_hoverRect.zw;
    bool inside = px.x >= r0.x && px.x <= r1.x && px.y >= r0.y && px.y <= r1.y;
    if (inside) {
      float edgeDist = min(min(px.x - r0.x, r1.x - px.x), min(px.y - r0.y, r1.y - px.y));
      float rim = 1.0 - smoothstep(0.0, 2.2, edgeDist);
      color = mix(color, vec3(0.95, 0.98, 1.0), 0.16 + rim * 0.72);
    }
  }

  outColor = vec4(clamp(color, 0.0, 1.0), 1.0);
}
`;

function compileShader(
  gl: WebGL2RenderingContext,
  type: number,
  source: string
): WebGLShader | null {
  const shader = gl.createShader(type);
  if (!shader) return null;
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    gl.deleteShader(shader);
    return null;
  }
  return shader;
}

export function createWebGL2CushionRenderer(
  canvas: HTMLCanvasElement
): WebGL2CushionRenderer | null {
  let gl: WebGL2RenderingContext | null = null;
  try {
    gl = canvas.getContext('webgl2', { antialias: false, alpha: false });
  } catch {
    return null;
  }
  if (!gl) return null;

  const vs = compileShader(gl, gl.VERTEX_SHADER, VERT_SRC);
  const fs = compileShader(gl, gl.FRAGMENT_SHADER, FRAG_SRC);
  if (!vs || !fs) return null;

  const program = gl.createProgram();
  if (!program) return null;
  gl.attachShader(program, vs);
  gl.attachShader(program, fs);
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    gl.deleteProgram(program);
    return null;
  }

  const vao = gl.createVertexArray();
  const vbo = gl.createBuffer();
  const tex = gl.createTexture();
  if (!vao || !vbo || !tex) return null;

  gl.bindVertexArray(vao);
  gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
  gl.bufferData(
    gl.ARRAY_BUFFER,
    new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]),
    gl.STATIC_DRAW
  );
  const loc = gl.getAttribLocation(program, 'a_pos');
  gl.enableVertexAttribArray(loc);
  gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);

  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);

  const uTex = gl.getUniformLocation(program, 'u_tex');
  const uRes = gl.getUniformLocation(program, 'u_resolution');
  const uCursor = gl.getUniformLocation(program, 'u_cursor');
  const uHoverRect = gl.getUniformLocation(program, 'u_hoverRect');
  const uHasHover = gl.getUniformLocation(program, 'u_hasHover');

  return {
    uploadTexture(buffer, width, height) {
      if (!gl) return;
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.texImage2D(
        gl.TEXTURE_2D,
        0,
        gl.RGBA,
        width,
        height,
        0,
        gl.RGBA,
        gl.UNSIGNED_BYTE,
        buffer
      );
    },
    draw(width, height, cursorX, cursorY, hoverRect) {
      if (!gl) return;
      gl.viewport(0, 0, width, height);
      gl.useProgram(program);
      gl.bindVertexArray(vao);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.uniform1i(uTex, 0);
      gl.uniform2f(uRes, width, height);
      gl.uniform2f(uCursor, cursorX, cursorY);
      if (hoverRect) {
        gl.uniform4f(uHoverRect, hoverRect[0], hoverRect[1], hoverRect[2], hoverRect[3]);
        gl.uniform1f(uHasHover, 1.0);
      } else {
        gl.uniform4f(uHoverRect, 0, 0, 0, 0);
        gl.uniform1f(uHasHover, 0.0);
      }
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    },
    dispose() {
      if (!gl) return;
      gl.deleteTexture(tex);
      gl.deleteBuffer(vbo);
      gl.deleteVertexArray(vao);
      gl.deleteProgram(program);
      gl.deleteShader(vs);
      gl.deleteShader(fs);
    },
  };
}
