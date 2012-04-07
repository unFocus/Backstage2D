package backstage2d.display 
{
	import backstage2d.display.spritesheets.SpriteNode;
	import flash.display.Bitmap;
	import flash.display.BitmapData;
	import flash.display.IBitmapDrawable;
	import flash.display.Sprite;
	import flash.geom.Matrix;
	import flash.geom.Matrix3D;
	import flash.geom.Point;
	
	import backstage2d.namespaces.bs2d;
	use namespace bs2d;
	
	/**
	 * A base class for all scene elements.
	 * @author Kevin Newman
	 */
	public class Actor implements IDraw
	{
		public var x:Number = 0;
		public var y:Number = 0;
		
		public var width:Number = 0;
		public var height:Number = 0;
		
		public var rotation:Number = 0;
		public var scaleX:Number = 1;
		public var scaleY:Number = 1;
		
		public var registration:Point;
		
		public var quality:String = "auto";
		
		public var layer:Layer// = null;
		
		public var visible:Boolean = true;
		
		public function set widthWithScale( widthTransformed:Number ):void {
			scaleX = width / widthTransformed;
		}
		public function get widthWithScale():Number {
			return width * scaleX;
		}
		
		public function set heightWithScale( heightTranformed:Number ):void {
			scaleY = height / heightTranformed;
		}
		public function get heightWithScale():Number {
			return height * scaleY;
		}
		
		public var transparency:Boolean = true;
		public var fillColor:uint// = 0;
		public var opacity:Number = 1;
		
		bs2d var texID:int = -1;
		bs2d function set texNode( node:SpriteNode ):void { }
		bs2d function get texNode():SpriteNode {
			return null;
		}
		
		function Actor( width:Number = 0, height:Number = 0 )
		{
			this.width = width;
			this.height = height;
			
			// registration defaults to centered
			this.registration = new Point( width / 2, height / 2 );
			
			this.scaleX = 0 < width? -1: 1;
			this.scaleY = 0 < height? -1: 1;
		}
		
		/**
		 * By default this makes an empty square. This method should be overridden by children.
		 * @return BitmapData The generated bitmapData asset for the Actor.
		 */
		public function draw( canvas:BitmapData, transform:Matrix = null ):BitmapData
		{
			//throw new Error( "You must override the draw method in child class" );
			return canvas;
		}
	}
}
