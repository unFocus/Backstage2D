package backstage2d.render.stage3d
{
	import backstage2d.display.spritesheets.SpriteSheet;
	import com.unfocus.signalslite.SignalLite;
    import flash.display.Bitmap;
    import flash.display.BitmapData;
    import flash.display.Stage;
    import flash.display3D.Context3D;
    import flash.display3D.Context3DTextureFormat;
    import flash.display3D.IndexBuffer3D;
    import flash.display3D.textures.Texture;
    import flash.geom.Point;
    import flash.geom.Rectangle;
    import flash.geom.Matrix;
	
	import backstage2d.display.Actor;
    
	import backstage2d.namespaces.bs2d;
	use namespace bs2d;
	
	final internal class TextureAtlas extends SpriteSheet
	{
		internal var texture:Texture;
		
		public const made:SignalLite = new SignalLite();
		
		public function TextureAtlas( width:int, height:int )
		{
			super( width, height );
			// :TODO: enforce a power of two size (round it up);
		}
		
		/*override public function make( actors:Vector.<Actor> ):BitmapData
		{
			return super.make( actors );
		}*/
		
		public function upload( context3D:Context3D ):void
        {
			if ( ! ss ) {
				trace( "nothing to upload...call make first");
			}
			
			if ( texture == null ) {
				texture = context3D.createTexture(
					ss.width, ss.height, Context3DTextureFormat.BGRA, false
				);
			}
			
			texture.uploadFromBitmapData( ss );
			
            // Courtesy of Starling: let's generate mipmaps
            var currentWidth:int = ss.width >> 1;
            var currentHeight:int = ss.height >> 1;
            var level:int = 1;
            var canvas:BitmapData = new BitmapData(currentWidth, currentHeight, true, 0);
            var transform:Matrix = new Matrix(.5, 0, 0, .5);
            
            while ( currentWidth >= 1 || currentHeight >= 1 ) {
                canvas.fillRect(new Rectangle(0, 0, Math.max(currentWidth,1), Math.max(currentHeight,1)), 0);
                canvas.draw(ss, transform, null, null, null, true);
                texture.uploadFromBitmapData(canvas, level++);
                transform.scale(0.5, 0.5);
                currentWidth = currentWidth >> 1;
                currentHeight = currentHeight >> 1;
            }
        }
    }
}
