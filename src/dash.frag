uniform float dashLength;
uniform float dashPeriod;
uniform float dashPhase;

in vec2 uvs;
in vec4 col;

layout (location = 0) out vec4 outColor;

void main()
{
    // Discarded, not drawn clear: a gap must write no depth either.
    if (mod(uvs.x - dashPhase, dashPeriod) > dashLength) {
        discard;
    }
    // Already linear: instance colors are converted on upload.
    outColor = col;
    outColor.rgb = color_mapping(outColor.rgb);
}
