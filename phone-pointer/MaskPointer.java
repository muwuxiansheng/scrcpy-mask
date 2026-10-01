import java.io.*;
import java.lang.reflect.*;

/** Shell-launched display-only pointer. It never injects input or owns window focus. */
public final class MaskPointer {
    static Object layer, surface;
    static Class<?> layerClass, transactionClass;
    static Object call(Object target, String name, Class<?>[] types, Object... args) throws Exception {
        Method m = target.getClass().getMethod(name, types);
        return m.invoke(target, args);
    }
    static Object transaction() throws Exception { return transactionClass.getConstructor().newInstance(); }
    static void finish(Object tx) throws Exception { call(tx, "apply", new Class<?>[0]); call(tx, "close", new Class<?>[0]); }
    static void hide() throws Exception {
        if (layer == null) return;
        Object tx = transaction(); call(tx, "hide", new Class<?>[]{layerClass}, layer); finish(tx);
    }
    static void create() throws Exception {
        if (layer != null) return;
        layerClass = Class.forName("android.view.SurfaceControl");
        transactionClass = Class.forName("android.view.SurfaceControl$Transaction");
        Object builder = Class.forName("android.view.SurfaceControl$Builder").getConstructor().newInstance();
        call(builder, "setName", new Class<?>[]{String.class}, "ScrcpyMaskDevicePointer");
        call(builder, "setBufferSize", new Class<?>[]{int.class, int.class}, 64, 72);
        call(builder, "setFormat", new Class<?>[]{int.class}, -3);
        layer = call(builder, "build", new Class<?>[0]);
        Class<?> sc = Class.forName("android.view.Surface");
        surface = sc.getConstructor().newInstance();
        call(surface, "copyFrom", new Class<?>[]{layerClass}, layer);
        Class<?> rect = Class.forName("android.graphics.Rect");
        Object canvas = call(surface, "lockCanvas", new Class<?>[]{rect}, (Object)null);
        Class<?> mode = Class.forName("android.graphics.PorterDuff$Mode");
        call(canvas, "drawColor", new Class<?>[]{int.class, mode}, 0, mode.getField("CLEAR").get(null));
        Class<?> pathClass = Class.forName("android.graphics.Path");
        Object path = pathClass.getConstructor().newInstance();
        call(path, "moveTo", new Class<?>[]{float.class,float.class}, 2f,2f);
        float[][] points = {{2,46},{14,34},{24,54},{32,49},{22,31},{42,31}};
        for (float[] p : points) call(path, "lineTo", new Class<?>[]{float.class,float.class}, p[0],p[1]);
        call(path, "close", new Class<?>[0]);
        Class<?> paintClass = Class.forName("android.graphics.Paint");
        Object paint = paintClass.getConstructor(int.class).newInstance(1);
        call(paint, "setColor", new Class<?>[]{int.class}, 0xffffffff);
        call(canvas, "drawPath", new Class<?>[]{pathClass,paintClass}, path,paint);
        Class<?> style = Class.forName("android.graphics.Paint$Style");
        call(paint, "setStyle", new Class<?>[]{style}, style.getField("STROKE").get(null));
        call(paint, "setStrokeWidth", new Class<?>[]{float.class}, 3f);
        call(paint, "setColor", new Class<?>[]{int.class}, 0xff000000);
        call(canvas, "drawPath", new Class<?>[]{pathClass,paintClass}, path,paint);
        call(surface, "unlockCanvasAndPost", new Class<?>[]{Class.forName("android.graphics.Canvas")}, canvas);
        Object tx = transaction();
        call(tx, "setLayer", new Class<?>[]{layerClass,int.class}, layer,Integer.MAX_VALUE-10);
        call(tx, "setLayerStack", new Class<?>[]{layerClass,int.class}, layer,0);
        finish(tx);
    }
    static void show(float x, float y) throws Exception {
        create(); Object tx = transaction();
        call(tx, "setPosition", new Class<?>[]{layerClass,float.class,float.class}, layer,x-2,y-2);
        call(tx, "show", new Class<?>[]{layerClass}, layer); finish(tx);
    }
    public static void main(String[] args) throws Exception {
        Class<?> serverClass = Class.forName("android.net.LocalServerSocket");
        Object server = serverClass.getConstructor(String.class).newInstance("mask-device-pointer");
        System.out.println("MaskPointer ready");
        while (true) {
            Object client = call(server,"accept",new Class<?>[0]);
            try {
                BufferedReader reader = new BufferedReader(new InputStreamReader((InputStream)call(client,"getInputStream",new Class<?>[0]), "UTF-8"));
                PrintWriter writer = new PrintWriter((OutputStream)call(client,"getOutputStream",new Class<?>[0]),true);
                String line;
                while ((line=reader.readLine()) != null) {
                    try {
                        String[] parts=line.split(" ");
                        if (parts[0].equals("HIDE")) hide();
                        else if (parts[0].equals("SHOW") && parts.length==3) show(Float.parseFloat(parts[1]),Float.parseFloat(parts[2]));
                        else throw new IllegalArgumentException("Unknown command");
                        writer.println("OK");
                    } catch (Exception e) { writer.println("ERROR " + e); e.printStackTrace(); }
                }
            } finally { hide(); call(client,"close",new Class<?>[0]); }
        }
    }
}
