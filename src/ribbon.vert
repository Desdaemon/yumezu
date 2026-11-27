// A connection as a flat strip facing the eye: the instance's x axis runs from one world to the
// other, and the y axis length is half the strip's width.
uniform mat4 viewProjection;
uniform vec3 eye;
// Past `fadeWholeWithin` of the origin a line dims to 1/e per `fadeLength`; zero disables the fade.
uniform float fadeWholeWithin;
uniform float fadeLength;
in vec3 position;
in vec4 row1;
in vec4 row2;
in vec4 row3;

#ifdef USE_UVS
#ifdef USE_INSTANCE_TEXTURE_TRANSFORMATION
in vec3 tex_transform_row1;
in vec3 tex_transform_row2;
#endif
in vec2 uv_coordinates;
out vec2 uvs;
#endif

#ifdef USE_INSTANCE_COLORS
in vec4 instance_color;
#endif

out vec3 pos;
out vec4 col;
flat out int instance_id;

float brightness_at(vec3 at)
{
    if (fadeLength <= 0.0) {
        return 1.0;
    }
    return exp(-max(length(at) - fadeWholeWithin, 0.0) / fadeLength);
}

void main()
{
    vec3 from = vec3(row1.w, row2.w, row3.w);
    vec3 span = vec3(row1.x, row2.x, row3.x);
    float half_width = length(vec3(row1.y, row2.y, row3.y));
    vec3 along = from + span * position.x;
    vec3 across = cross(span, eye - along);
    // A zero-scaled instance, or one seen end on, collapses rather than dividing by zero.
    vec3 side = across * (half_width / max(length(across), 1e-20));
    pos = along + side * position.y;
    gl_Position = viewProjection * vec4(pos, 1.0);

#ifdef USE_UVS
#ifdef USE_INSTANCE_TEXTURE_TRANSFORMATION
    mat3 texTransform;
    texTransform[0] = vec3(tex_transform_row1.x, tex_transform_row2.x, 0.0);
    texTransform[1] = vec3(tex_transform_row1.y, tex_transform_row2.y, 0.0);
    texTransform[2] = vec3(tex_transform_row1.z, tex_transform_row2.z, 1.0);
    uvs = (texTransform * vec3(uv_coordinates, 1.0)).xy;
#else
    uvs = uv_coordinates;
#endif
#endif

    col = vec4(1.0);
#ifdef USE_INSTANCE_COLORS
    col *= instance_color;
#endif
    // Interpolated between the ends: the chord passes nearer the origin than either end.
    col.rgb *= mix(brightness_at(from), brightness_at(from + span), position.x);
    instance_id = gl_InstanceID;
}
