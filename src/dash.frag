uniform float dashLength;
uniform float dashPeriod;
uniform float dashPhase;

in vec2 uvs;
in vec4 col;

layout (location = 0) out vec4 outColor;

void main()
{
    // `uvs.x` is how far along the connection this fragment sits, in world units: see
    // `dash_line`. Discarded rather than drawn clear, a gap having to write no depth either.
    if (mod(uvs.x - dashPhase, dashPeriod) > dashLength) {
        discard;
    }
    // Already linear: an instance color is converted on its way into the buffer.
    outColor = col;
    outColor.rgb = color_mapping(outColor.rgb);
}
