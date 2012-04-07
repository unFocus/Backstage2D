package backstage2d.display.classic 
{
	import flash.display.BitmapData;
	import flash.display.IBitmapDrawable;
	import flash.display.Sprite;
	import flash.geom.Matrix;
	import flash.geom.Point;
	import backstage2d.display.Actor;
	
	/**
	 * An actor for rendering a standard Sprite.
	 * @author Kevin Newman
	 */
	public class BSSprite extends Actor 
	{
		protected var sprite:Sprite;
		
		public function BSSprite( sprite:Sprite ) 
		{
			// :TODO: Assume classic display objects' sizes are specified
			// at 72 DPI on the stage. Then do the calculations to convert
			// for the current DPI and unit of measurement.
			super( sprite.width, sprite.height );
			this.sprite = sprite;
			this.registration = new Point(
				-sprite.transform.pixelBounds.x,
				-sprite.transform.pixelBounds.y
			);
		}
		
		/**
		 * By default this makes an empty square. This method should be overridden by children.
		 * @return BitmapData The generated bitmapData asset for the Actor.
		 */
		override public function draw( canvas:BitmapData, transform:Matrix = null ):BitmapData
		{
			canvas.draw( sprite, transform, null, null, null, true );
			return canvas;
		}
	}
}
