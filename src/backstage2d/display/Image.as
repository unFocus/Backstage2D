package backstage2d.display 
{
	import backstage2d.display.spritesheets.SpriteNode;
	import flash.display.BitmapData;
	import flash.geom.Matrix;
	import flash.geom.Point;
	
	import backstage2d.display.spritesheets.SpriteRegistry;
	
	import backstage2d.namespaces.bs2d;
	use namespace bs2d;
	
	/**
	 * ...
	 * @author Kevin Newman
	 */
	public class Image extends Actor
	{
		protected var data:BitmapData;
		
		/**
		 * Constructor
		 * @param	bitmapData The bitmapData to use for the image.
		 */
		public function Image( bitmapData:BitmapData )
		{
			super( bitmapData.width, bitmapData.height );
			data = bitmapData;
		}
		
		override bs2d function set texNode( node:SpriteNode ):void {
			SpriteRegistry.instance[ data ] = node;
		}
		override bs2d function get texNode():SpriteNode {
			return SpriteRegistry.instance[ data ];
		}
		
		/**
		 * Returns the cached BitmapData instance passed into the constructor. Image
		 * cache cannot be invalidated.
		 * @return
		 */
		public override function draw( canvas:BitmapData, transform:Matrix = null ):BitmapData
		{
			/*if ( !canvas ) {
				canvas = new BitmapData( width, height );
			}*/
			
			// if the user changes the width and height manually, this will force those values.
			// :TODO: Update this for DPI and unit of measurement.
			// :TODO: Make sure we don't upscale (let the GPU do it).
			if ( !transform && ( data.width != width || data.height != height ) )
			{
				transform = new Matrix();
				transform.scale( width / data.width, height / data.height );
			}
			
			//if ( transform )
			//{
				canvas.draw( data, transform, null, null, null, true );
			// :TODO: Consider if it's worth it to enable a fast path copyPixels method.
			//}
			//else
			//	canvas.copyPixels( data, data.rect, new Point( transform.tx, transform.ty ) );
			
			return canvas;
		}
		
	}
}
